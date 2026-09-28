//! Отрисовка кадра. Чистые функции от `View` — ничего не мутируют.

use super::cube::{draw_cube, draw_cubes, draw_piece, draw_piece_shadow, CubeDraw, CubeStyle};
use super::fx::*;
use super::layout::{inset, offset, scaled_about_center};
use super::overlay::{self, Card};
use super::theme::{self, hex};
use super::world;
use super::{Overlay, Screen, View};
use crate::anim::{self, clamp01};
use crate::game::{Ability, Game, MAX_ENERGY, SIZE};
use syngui::mss::{TextAlign, TextDecoration};
use syngui::prelude::*;
use syngui::render::display_list::Border;

// ── текст ──────────────────────────────────────────────────────────────────

pub fn text(list: &mut DisplayList, s: &str, rect: Rect, color: Color, size: f32, weight: u16, align: TextAlign) {
    list.push_text_styled(s, rect, color, size, align, TextDecoration::None, weight, Some(theme::FONT.to_string()));
}

/// Текст в одну строку без переноса (лишнее обрезается клипом).
pub fn text_nowrap(list: &mut DisplayList, s: &str, rect: Rect, color: Color, size: f32, weight: u16, align: TextAlign) {
    list.push_text_styled_singleline(s, rect, color, size, align, TextDecoration::None, weight, Some(theme::FONT.to_string()));
}

pub fn text_shadowed(list: &mut DisplayList, s: &str, rect: Rect, color: Color, size: f32, weight: u16, align: TextAlign) {
    let sh = hex(theme::TEXT_SHADOW).with_alpha(0.55 * color.a);
    text(list, s, offset(rect, 0.0, size * 0.06), sh, size, weight, align);
    text(list, s, rect, color, size, weight, align);
}

pub fn icon(list: &mut DisplayList, glyph: &str, rect: Rect, color: Color, size: f32) {
    list.push_text_styled(glyph, rect, color, size, TextAlign::CENTER, TextDecoration::None, 400, None);
}

// ── кадр ───────────────────────────────────────────────────────────────────

pub fn draw(v: &View, list: &mut DisplayList) {
    let t = v.screen_t();
    match v.screen {
        Screen::Start => {
            // игра под стартовой страницей ещё видна, пока та проявляется
            if t < 1.0 {
                draw_game(v, list);
            }
            super::start::draw(v, list, anim::ease_out_cubic(t), (1.0 - anim::ease_out_cubic(t)) * 30.0);
        }
        Screen::Game => {
            draw_game(v, list);
            if t < 1.0 {
                let k = anim::ease_out_cubic(t);
                super::start::draw(v, list, 1.0 - k, -k * 60.0);
            }
        }
    }
}

fn draw_game(v: &View, list: &mut DisplayList) {
    let game = v.game();
    background(v, list);
    let (sx, sy) = v.fx.shake_offset(v.now);
    hud(v, &game, list);
    board(v, &game, list, sx, sy);
    skin_chip(v, list);
    tray(v, &game, list);
    abilities(v, &game, list);
    v.fx.particles.draw(list);
    floats(v, list);
    over_hint(v, list);
    banner(v, list);
    for f in &v.fx.screen_flashes {
        let t = clamp01((v.now - f.born) / FLASH_DUR);
        let a = f.strength * (1.0 - t).powi(2);
        if a > 0.002 {
            list.push_rect(v.geo.bounds, f.color.with_alpha(a), [0.0; 4]);
        }
    }
    overlays(v, &game, list);
}

fn background(v: &View, list: &mut DisplayList) {
    // Фон — на всё окно, включая safe area (приложение edge-to-edge).
    let b = v.geo.bounds;
    let pal = v.palette();
    list.push_gradient_rect(
        b,
        Gradient::Linear { angle_deg: 180.0, stops: vec![ColorStop::new(pal.top, 0.0), ColorStop::new(pal.bottom, 1.0)] },
        [0.0; 4],
    );
    // частицы мира: во время смены — обоих миров с перекрёстной прозрачностью
    let wf = v.world;
    let k = wf.t(v.now);
    let (from, to) = (&world::WORLDS[wf.from], &world::WORLDS[wf.to]);
    if wf.from != wf.to && k < 1.0 {
        world::draw_ambient(list, from.ambient, b, v.now, 1.0 - k, hex(from.accent));
    }
    world::draw_ambient(list, to.ambient, b, v.now, if wf.from != wf.to { k } else { 1.0 }, hex(to.accent));
}

fn hud(v: &View, game: &Game, list: &mut DisplayList) {
    let g = &v.geo;
    let stats = v.stats();
    let best = stats.best_of(v.variant, game.difficulty).max(game.scoring.score);

    // рекорд
    let gold = hex(theme::GOLD);
    let flash = v.fx.best_flash_born.map(|t| anim::punch(clamp01((v.now - t) / 0.8))).unwrap_or(0.0);
    let icon_rect = Rect::new(g.best.origin, Size::new(36.0, g.best.height()));
    icon(list, theme::ICON_TROPHY, icon_rect, gold.lerp(&Color::WHITE, flash * 0.6), 27.0 + flash * 6.0);
    let num_rect = Rect::new(Point::new(g.best.origin.x + 40.0, g.best.origin.y), Size::new(g.best.width() - 40.0, g.best.height()));
    text_shadowed(list, &best.to_string(), num_rect, gold.lerp(&Color::WHITE, flash * 0.5), 25.0 + flash * 5.0, 800, TextAlign::LEFT | TextAlign::VCENTER);
    // метка сложности под рекордом
    let dr = Rect::new(Point::new(g.best.origin.x + 42.0, g.best.origin.y + g.best.height() - 6.0), Size::new(120.0, 14.0));
    text(list, game.difficulty.name(), dr, hex(theme::TEXT_DIM).with_alpha(0.85), 10.5, 800, TextAlign::LEFT | TextAlign::VCENTER);

    // комбо-пилюля по центру верхней строки; с ×3 — «огонь»
    if game.scoring.combo >= 2 && !game.over {
        let pill = g.combo_pill();
        let fire = game.scoring.combo >= 3;
        let accent = if fire { gold.lerp(&hex("#ff7a2f"), 0.45) } else { gold };
        let pulse = ((v.now * (if fire { 9.0 } else { 5.0 })).sin() * 0.5 + 0.5) * if fire { 0.16 } else { 0.08 };
        if fire {
            list.push_rect(inset(pill, -5.0), accent.with_alpha(0.10 + pulse * 0.5), [19.0; 4]);
        }
        list.push_rect_bordered(pill, accent.with_alpha(0.16 + pulse), [14.0; 4], Border { width: 1.5, color: accent.with_alpha(0.8) });
        let inner = Rect::new(Point::new(pill.origin.x + 8.0, pill.origin.y), Size::new(22.0, pill.height()));
        icon(list, if fire { theme::ICON_FIRE } else { theme::ICON_BOLT }, inner, accent, 18.0 + pulse * 10.0);
        let tr = Rect::new(Point::new(pill.origin.x + 30.0, pill.origin.y), Size::new(pill.width() - 34.0, pill.height()));
        text(list, &format!("КОМБО ×{}", game.scoring.combo), tr, accent.lerp(&Color::WHITE, 0.2), 14.0, 800, TextAlign::CENTER);
    }

    level_ring(v, game, list);

    // кнопка меню
    let hot = g.menu_btn.contains(v.pointer) && v.overlay().is_none();
    list.push_rect(g.menu_btn, Color::WHITE.with_alpha(if hot { 0.24 } else { 0.12 }), [20.0; 4]);
    icon(list, theme::ICON_SETTINGS, g.menu_btn, Color::WHITE.with_alpha(0.95), 23.0);

    // счёт
    let punch = anim::punch(clamp01((v.now - v.fx.score_punch_born) / 0.5));
    let size = 48.0 * (1.0 + 0.18 * punch);
    let shown = v.fx.score_shown.round().max(0.0) as i64;
    let fire = if game.scoring.combo >= 3 && !game.over { 0.35 + 0.15 * (v.now * 6.0).sin() } else { 0.0 };
    let color = Color::WHITE.lerp(&gold, (punch * 0.7).max(fire));
    text_shadowed(list, &shown.to_string(), g.score, color, size, 800, TextAlign::CENTER);
}

fn cell_radius(cell: f32) -> [f32; 4] {
    [cell * 0.13; 4]
}

/// Рамка поля «Эльвиры»: золотая кромка, завитки по углам, виньетка.
fn elvira_frame(v: &View, list: &mut DisplayList, panel: Rect, radius: [f32; 4]) {
    let gold = hex("#d9b25a");
    let pulse = (v.now * 0.8).sin() * 0.5 + 0.5;
    list.push_outline(inset(panel, -3.0), gold.with_alpha(0.35 + 0.15 * pulse), 1.5, 0.0, [radius[0] + 3.0; 4]);
    list.push_inner_shadow(panel, Color::from_srgb(0, 0, 0, 0.30), 40.0, (0.0, 0.0), radius);
    // завитки по углам — короткие дуги
    let (x0, y0, x1, y1) = (panel.origin.x, panel.origin.y, panel.origin.x + panel.width(), panel.origin.y + panel.height());
    let r = 14.0;
    for (cx, cy, sx, sy) in [(x0 + 12.0, y0 + 12.0, 1.0, 1.0), (x1 - 12.0, y0 + 12.0, -1.0, 1.0), (x0 + 12.0, y1 - 12.0, 1.0, -1.0), (x1 - 12.0, y1 - 12.0, -1.0, -1.0)] {
        let pts: Vec<[f32; 2]> = (0..=10)
            .map(|i| {
                let u = i as f32 / 10.0 * std::f32::consts::FRAC_PI_2 * 1.5;
                [cx + sx * (r * 0.4 + u.cos() * r * (1.0 - u * 0.25)), cy + sy * (r * 0.4 + u.sin() * r * (1.0 - u * 0.25))]
            })
            .collect();
        list.push_line_strip(pts, gold.with_alpha(0.55), 1.6);
        list.push_rect(Rect::new(Point::new(cx - 2.0, cy - 2.0), Size::new(4.0, 4.0)), gold.with_alpha(0.8), [2.0; 4]);
    }
}

fn board(v: &View, game: &Game, list: &mut DisplayList, sx: f32, sy: f32) {
    let g = &v.geo;
    let panel = offset(g.board, sx, sy);
    let radius = [18.0; 4];
    let cell = g.cell;

    // свечение и тень
    let pal = v.palette();
    let elv = v.variant.is_elvira();
    list.push_shadow(panel, pal.glow.with_alpha(0.30), 42.0, (0.0, 0.0), radius);
    list.push_shadow(panel, Color::from_srgb(0, 0, 0, 0.42), 26.0, (0.0, 14.0), radius);
    list.push_rect_bordered(panel, pal.panel, radius, Border { width: 2.0, color: pal.edge.with_alpha(0.85) });
    list.push_inner_shadow(panel, Color::from_srgb(0, 0, 0, 0.35), 16.0, (0.0, 6.0), radius);
    if elv {
        elvira_frame(v, list, panel, radius);
    }
    if game.scoring.combo >= 3 && !game.over {
        let pulse = (v.now * 5.0).sin() * 0.5 + 0.5;
        let gold = hex(theme::GOLD);
        list.push_outline(inset(panel, 1.0), gold.with_alpha(0.35 + 0.35 * pulse), 2.5, 0.0, radius);
        list.push_outline(inset(panel, -4.0), gold.with_alpha(0.10 + 0.12 * pulse), 4.0, 0.0, [22.0; 4]);
    }

    // пустые клетки («Эльвира» — с тонким светлым ободком)
    let empty = pal.empty;
    for r in 0..SIZE {
        for c in 0..SIZE {
            let rect = offset(g.cube_rect(r, c), sx, sy);
            list.push_rect(rect, empty, cell_radius(cell));
            if elv {
                list.push_outline(inset(rect, 0.5), Color::WHITE.with_alpha(0.05), 1.0, 0.0, cell_radius(cell));
            }
        }
    }

    // превью (drag или клавиатура)
    let mut ghost_cells: Vec<(usize, usize)> = Vec::new();
    let mut preview_lines: Option<(Vec<usize>, Vec<usize>, Color)> = None;
    let target = v
        .drag_target(game)
        .or_else(|| v.kb.map(|k| (k.slot, (k.r, k.c))));
    if let Some((slot, (r, c))) = target {
        if let Some(piece) = game.piece(slot) {
            let valid = game.board.can_place(&piece.shape, r, c);
            let base = v.piece_look(piece).base;
            for &(dr, dc) in piece.shape.cells() {
                let (rr, cc) = (r + dr as i32, c + dc as i32);
                if rr < 0 || cc < 0 || rr >= SIZE as i32 || cc >= SIZE as i32 {
                    continue;
                }
                let rect = offset(g.cube_rect(rr as usize, cc as usize), sx, sy);
                if valid {
                    list.push_rect(rect, base.with_alpha(0.42), cell_radius(cell));
                    list.push_rect(inset(rect, cell * 0.16), base.lighten(0.3).with_alpha(0.35), cell_radius(cell * 0.6));
                    ghost_cells.push((rr as usize, cc as usize));
                } else {
                    list.push_rect(rect, hex(theme::DANGER).with_alpha(0.22), cell_radius(cell));
                }
            }
            if valid {
                let (rows, cols) = game.board.preview_lines(&piece.shape, r, c);
                if !rows.is_empty() || !cols.is_empty() {
                    preview_lines = Some((rows, cols, base));
                }
            }
        }
    }

    // цель способности
    if let Some(a) = v.targeting {
        if let Some((tr, tc)) = g.point_to_cell(v.pointer) {
            let hot = hex("#ff7a2f");
            let pulse = (v.now * 8.0).sin() * 0.5 + 0.5;
            let rad = if a == Ability::Bomb { 1 } else { 0 };
            for dr in -rad..=rad {
                for dc in -rad..=rad {
                    let (rr, cc) = (tr + dr, tc + dc);
                    if rr < 0 || cc < 0 || rr >= SIZE as i32 || cc >= SIZE as i32 {
                        continue;
                    }
                    let rect = offset(g.cube_rect(rr as usize, cc as usize), sx, sy);
                    list.push_rect(rect, hot.with_alpha(0.28 + 0.14 * pulse), cell_radius(cell));
                }
            }
            let side = (rad * 2 + 1) as f32 * cell;
            let r0 = g.cell_rect((tr - rad).max(0) as usize, (tc - rad).max(0) as usize);
            let x = (g.grid.x + (tc - rad) as f32 * cell).max(g.grid.x);
            let y = (g.grid.y + (tr - rad) as f32 * cell).max(g.grid.y);
            let _ = r0;
            let w = side.min(g.grid.x + g.grid_side() - x);
            let h = side.min(g.grid.y + g.grid_side() - y);
            list.push_outline(inset(Rect::new(Point::new(x + sx, y + sy), Size::new(w, h)), 1.5), hot.with_alpha(0.7 + 0.3 * pulse), 2.0, 0.0, [cell * 0.2; 4]);
        }
    }

    // кубики поля (пакетом: текстуры одним батчем, потом скосы и блики)
    let grey = hex("#5a5f80");
    let panel_col = pal.panel;
    let mut draws: Vec<CubeDraw> = Vec::with_capacity(SIZE * SIZE);
    for r in 0..SIZE {
        for c in 0..SIZE {
            let val = game.board.get(r, c);
            if val == 0 {
                continue;
            }
            let mut rect = offset(g.cube_rect(r, c), sx, sy);
            let look = v.cell_look(val, r, c);
            let mut style = v.cube_style(&look, (r * SIZE + c) as u32);
            let sf = v.skin_flash(world::cell_delay(r, c));
            if sf > 0.0 {
                rect = scaled_about_center(rect, 1.0 + 0.12 * sf);
                style.flash += 0.6 * sf;
            }
            let rs = v.fx.ripple_scale(v.now, r, c);
            if rs > 1.001 {
                rect = scaled_about_center(rect, rs);
                style.flash += (rs - 1.0) * 2.0;
            }
            if let Some(p) = v.fx.pops.iter().find(|p| p.r == r && p.c == c) {
                let t = clamp01((v.now - p.born) / POP_DUR);
                let k = anim::punch(t);
                rect = scaled_about_center(rect, 1.0 + 0.18 * k);
                style.flash = 0.55 * (1.0 - anim::ease_out_cubic(t));
            }
            if let Some(t0) = v.fx.over_at {
                let k = clamp01((v.now - t0 - (r + c) as f32 * 0.04) / 0.3);
                if k > 0.0 {
                    // приглушить, но оставить цвета — видно, как легло поле
                    style.blend = Some((grey, k * 0.45));
                }
            } else {
                style.flash += 0.32 * Fx::sheen(v.now, r, c);
            }
            draws.push(CubeDraw { rect, base: look.base, style });
        }
    }

    // исчезающие кубики
    for vn in &v.fx.vanish {
        let t = (v.now - vn.born - vn.delay) / VANISH_DUR;
        let rect = offset(g.cube_rect(vn.r, vn.c), sx, sy);
        let look = v.cell_look(vn.color, vn.r, vn.c);
        let base = look.base;
        let seed = (vn.r * SIZE + vn.c) as u32;
        if t < 0.0 {
            let pre = clamp01(1.0 + t * 3.0);
            let style = CubeStyle { flash: 0.65 * pre, ..v.cube_style(&look, seed) };
            draws.push(CubeDraw { rect, base, style });
        } else if t < 1.0 {
            let k = anim::ease_in_cubic(t);
            let s = (1.0 - k).max(0.02);
            let mut style = CubeStyle { flash: (1.0 - t) * 0.6, ..v.cube_style(&look, seed) };
            style.blend = Some((panel_col, k * 0.8));
            draws.push(CubeDraw { rect: scaled_about_center(rect, s), base, style });
        }
    }
    draw_cubes(list, &draws);

    // вспышки линий
    for f in &v.fx.flashes {
        let t = clamp01((v.now - f.born) / LINE_FLASH_DUR);
        let a = (1.0 - t).powi(2) * 0.55;
        let rect = if f.is_row {
            let r0 = g.cell_rect(f.idx, 0);
            Rect::new(Point::new(r0.origin.x + sx, r0.origin.y + sy), Size::new(g.grid_side(), cell))
        } else {
            let c0 = g.cell_rect(0, f.idx);
            Rect::new(Point::new(c0.origin.x + sx, c0.origin.y + sy), Size::new(cell, g.grid_side()))
        };
        let grow = 1.0 + t * 0.12;
        list.push_rect(scaled_about_center(rect, grow), Color::WHITE.with_alpha(a), [cell * 0.2; 4]);
    }

    // контуры «приземления» поставленных кубиков
    for l in &v.fx.lands {
        let t = clamp01((v.now - l.born) / LAND_DUR);
        let k = anim::ease_out_cubic(t);
        let rect = scaled_about_center(offset(g.cube_rect(l.r, l.c), sx, sy), 1.0 + 0.45 * k);
        list.push_outline(rect, Color::WHITE.with_alpha(0.55 * (1.0 - t).powi(2)), 2.0 * (1.0 - t) + 0.6, 0.0, cell_radius(cell * (1.0 + k)));
    }

    // лучи по очищенным линиям: из точки очистки к обоим краям
    for bm in &v.fx.beams {
        let t = clamp01((v.now - bm.born) / BEAM_DUR);
        let k = anim::ease_out_cubic(t);
        let a = (1.0 - t).powf(1.5);
        let side = g.grid_side();
        let from = bm.from.clamp(0.0, SIZE as f32) * cell;
        let reach = side * 1.05 * k;
        let len = cell * 2.2;
        for dir in [-1.0f32, 1.0] {
            let head = (from + dir * reach).clamp(-len, side + len);
            let (x0, x1) = if dir > 0.0 { (head - len, head) } else { (head, head + len) };
            let x0c = x0.max(0.0);
            let x1c = x1.min(side);
            if x1c <= x0c {
                continue;
            }
            let (angle, stops) = if dir > 0.0 {
                (90.0, vec![ColorStop::new(bm.color.with_alpha(0.0), 0.0), ColorStop::new(Color::WHITE.with_alpha(0.85 * a), 1.0)])
            } else {
                (90.0, vec![ColorStop::new(Color::WHITE.with_alpha(0.85 * a), 0.0), ColorStop::new(bm.color.with_alpha(0.0), 1.0)])
            };
            let rect = if bm.is_row {
                let y = g.grid.y + bm.idx as f32 * cell + sy;
                Rect::new(Point::new(g.grid.x + x0c + sx, y + cell * 0.2), Size::new(x1c - x0c, cell * 0.6))
            } else {
                let x = g.grid.x + bm.idx as f32 * cell + sx;
                Rect::new(Point::new(x + cell * 0.2, g.grid.y + x0c + sy), Size::new(cell * 0.6, x1c - x0c))
            };
            let angle = if bm.is_row { angle } else { 180.0 };
            list.push_gradient_rect(rect, Gradient::Linear { angle_deg: angle, stops }, [cell * 0.3; 4]);
        }
    }

    // подсветка линий, которые заполнятся
    if let Some((rows, cols, base)) = preview_lines {
        let pulse = (v.now * 7.0).sin() * 0.5 + 0.5;
        let col = base.lerp(&Color::WHITE, 0.55).with_alpha(0.30 + 0.14 * pulse);
        for r in 0..SIZE {
            for c in 0..SIZE {
                if (rows.contains(&r) || cols.contains(&c)) && !ghost_cells.contains(&(r, c)) {
                    let rect = offset(g.cube_rect(r, c), sx, sy);
                    list.push_rect(rect, col, cell_radius(cell));
                }
            }
        }
        // тонкая рамка вокруг каждой линии
        let edge = base.lerp(&Color::WHITE, 0.7).with_alpha(0.55 + 0.25 * pulse);
        for &r in &rows {
            let r0 = g.cell_rect(r, 0);
            let rect = Rect::new(Point::new(r0.origin.x + sx, r0.origin.y + sy), Size::new(g.grid_side(), cell));
            list.push_outline(inset(rect, 1.5), edge, 2.0, 0.0, [cell * 0.2; 4]);
        }
        for &c in &cols {
            let c0 = g.cell_rect(0, c);
            let rect = Rect::new(Point::new(c0.origin.x + sx, c0.origin.y + sy), Size::new(cell, g.grid_side()));
            list.push_outline(inset(rect, 1.5), edge, 2.0, 0.0, [cell * 0.2; 4]);
        }
    }

    // ударные волны
    for sw in &v.fx.shocks {
        let t = clamp01((v.now - sw.born) / SHOCK_DUR);
        let k = anim::ease_out_cubic(t);
        let r = 12.0 + sw.max_r * k;
        let a = (1.0 - t).powi(2) * 0.85;
        let rect = Rect::new(Point::new(sw.center.x - r + sx, sw.center.y - r + sy), Size::new(r * 2.0, r * 2.0));
        list.push_outline(rect, sw.color.with_alpha(a), 2.5 + 7.0 * (1.0 - t), 0.0, [r; 4]);
        let r2 = r * 0.72;
        let rect2 = Rect::new(Point::new(sw.center.x - r2 + sx, sw.center.y - r2 + sy), Size::new(r2 * 2.0, r2 * 2.0));
        list.push_outline(rect2, Color::WHITE.with_alpha(a * 0.5), 1.5, 0.0, [r2; 4]);
    }
}

fn tray(v: &View, game: &Game, list: &mut DisplayList) {
    let g = &v.geo;
    let bg = v.palette().bottom;
    let tc = g.tray_cell;
    let gap = (tc * 0.045).max(0.8);

    for i in 0..3 {
        if let Some(ret) = v.fx.returning.filter(|r| r.slot == i) {
            if let Some(p) = game.piece(i) {
                let t = anim::ease_out_cubic(clamp01((v.now - ret.born) / RETURN_DUR));
                let to = g.piece_origin_in_slot(i, &p.shape);
                let o = Point::new(anim::lerp(ret.from.x, to.x, t), anim::lerp(ret.from.y, to.y, t));
                let c = anim::lerp(ret.from_cell, tc, t);
                let look = v.piece_look(p);
                draw_piece(list, &p.shape, o, c, (c * 0.045).max(0.8), 1.0, look.base, &v.cube_style(&look, 100 + i as u32));
            }
            continue;
        }
        if v.drag.map(|d| d.slot == i && d.active).unwrap_or(false) {
            continue;
        }
        let Some(p) = game.piece(i) else { continue };
        let origin = g.piece_origin_in_slot(i, &p.shape);
        let born = v.fx.tray_born[i];
        let t_in = clamp01((v.now - born) / TRAY_IN_DUR);
        if t_in <= 0.0 {
            continue;
        }
        let k_in = anim::ease_out_back(t_in);
        let bob = if v.fx.over_at.is_none() { ((v.now * 1.7 + i as f32 * 2.1).sin()) * 2.2 } else { 0.0 };
        let o = Point::new(origin.x, origin.y + (1.0 - k_in) * 46.0 + bob);
        let fits = game.board.fits_anywhere(&p.shape);
        let hover = v.fx.tray_hover[i];
        let selected = v.kb.map(|k| k.slot == i).unwrap_or(false);
        let look = v.piece_look(p);
        let mut style = v.cube_style(&look, 100 + i as u32);
        let sf = v.skin_flash(world::TRAY_DELAY + i as f32 * 0.06);
        style.flash += 0.6 * sf;
        let mut blend = 1.0 - anim::ease_out_cubic(t_in);
        if !fits && v.fx.over_at.is_none() {
            blend = blend.max(0.62);
        }
        if blend > 0.0 {
            style.blend = Some((bg, blend));
        }
        let over_k = v.fx.over_at.map(|t0| clamp01((v.now - t0) / 0.5)).unwrap_or(0.0);
        if over_k > 0.0 {
            style.blend = Some((bg, 0.25 * over_k));
        }
        if selected {
            let pr = g.piece_rect_in_slot(i, &p.shape);
            let hl = inset(pr, -10.0);
            list.push_rect(hl, Color::WHITE.with_alpha(0.14), [14.0; 4]);
            list.push_outline(hl, Color::WHITE.with_alpha(0.55), 1.5, 0.0, [14.0; 4]);
        }
        let scale = 1.0 + hover * 0.09;
        if hover > 0.01 && fits {
            let pr = scaled_about_center(g.piece_rect_in_slot(i, &p.shape), scale);
            list.push_rect(offset(inset(pr, -6.0), 0.0, 6.0 * hover), Color::from_srgb(0, 0, 0, 0.16 * hover), [12.0; 4]);
        }
        if v.variant.is_elvira() && fits && v.fx.over_at.is_none() {
            // мягкое свечение под фигурой цветом её материала
            let pr = g.piece_rect_in_slot(i, &p.shape);
            let glow = look.accent.unwrap_or(look.base).lerp(&look.base, 0.5);
            list.push_glow_shadow(offset(inset(pr, 2.0), 0.0, 4.0 + bob * 0.3), glow.with_alpha(0.10 + 0.05 * hover), 18.0, (0.0, 0.0), [10.0; 4]);
        }
        draw_piece(list, &p.shape, o, tc, gap, scale * (1.0 + 0.1 * sf), look.base, &style);
        if over_k > 0.0 && !fits {
            // фигура, которой некуда встать, — причина проигрыша
            let pulse = (v.now * 4.0 + i as f32).sin() * 0.5 + 0.5;
            let pr = inset(g.piece_rect_in_slot(i, &p.shape), -8.0);
            list.push_outline(pr, hex(theme::DANGER).with_alpha((0.55 + 0.35 * pulse) * over_k), 2.0, 0.0, [12.0; 4]);
        }
    }

    // перетаскиваемая фигура — поверх всего. Рисуется в оверлейном слое:
    // батчер иначе может увести её полигоны под подсветку линий.
    if let Some(d) = v.drag.filter(|d| d.active) {
        if let Some(p) = game.piece(d.slot) {
            let (origin, cell_now) = v.drag_origin(&d);
            let k = anim::ease_out_cubic(d.lift);
            let gap_now = (cell_now * 0.045).max(0.8);
            list.begin_overlay_absolute();
            draw_piece_shadow(list, &p.shape, origin, cell_now, gap_now, 0.30 + 0.15 * k, cell_now * 0.22 * k + 3.0);
            let valid = v.drag_target(game).is_some();
            let look = v.piece_look(p);
            let style = CubeStyle { flash: if valid { 0.08 } else { 0.0 }, ..v.cube_style(&look, 200 + d.slot as u32) };
            draw_piece(list, &p.shape, origin, cell_now, gap_now, 1.0, look.base, &style);
            list.end_overlay();
        }
    }
}

fn abilities(v: &View, game: &Game, list: &mut DisplayList) {
    let g = &v.geo;
    let gold = hex(theme::GOLD);
    let locked = v.input_locked_pub();
    // энергия
    let bar = g.energy_bar;
    list.push_rect(bar, Color::from_srgb(0, 0, 0, 0.35), [3.0; 4]);
    let frac = (game.energy as f32 / MAX_ENERGY as f32).clamp(0.0, 1.0);
    if frac > 0.0 {
        let fill = Rect::new(bar.origin, Size::new(bar.width() * frac, bar.height()));
        list.push_gradient_rect(
            fill,
            Gradient::Linear { angle_deg: 90.0, stops: vec![ColorStop::new(gold.darken(0.15), 0.0), ColorStop::new(gold.lighten(0.25), 1.0)] },
            [3.0; 4],
        );
        if frac >= 1.0 {
            let pulse = (v.now * 6.0).sin() * 0.5 + 0.5;
            list.push_outline(bar, gold.with_alpha(0.3 + 0.4 * pulse), 1.5, 1.0, [4.0; 4]);
        }
    }
    // деления по стоимости
    for a in Ability::ALL {
        let x = bar.origin.x + bar.width() * a.cost() as f32 / MAX_ENERGY as f32;
        list.push_rect(Rect::new(Point::new(x - 1.0, bar.origin.y - 2.0), Size::new(2.0, bar.height() + 4.0)), Color::WHITE.with_alpha(0.5), [1.0; 4]);
    }
    for (i, a) in Ability::ALL.iter().enumerate() {
        let rect = g.abil_btns[i];
        let can = game.can_use(*a) && !locked;
        let hot = rect.contains(v.pointer) && can;
        let selected = v.targeting == Some(*a);
        let bg = if selected {
            gold.with_alpha(0.9)
        } else if can {
            hex(if hot { theme::BTN_HOVER } else { theme::BTN })
        } else {
            Color::from_srgb(20, 24, 70, 0.45)
        };
        let r = offset(rect, 0.0, if hot && !selected { -2.0 } else { 0.0 });
        if can {
            list.push_rect(offset(r, 0.0, 3.0), Color::from_srgb(0, 0, 0, 0.25), [14.0; 4]);
        }
        list.push_gradient_rect(
            r,
            Gradient::Linear { angle_deg: 180.0, stops: vec![ColorStop::new(bg.lighten(0.12), 0.0), ColorStop::new(bg, 1.0)] },
            [14.0; 4],
        );
        list.push_outline(inset(r, 1.0), Color::WHITE.with_alpha(if can { 0.22 } else { 0.08 }), 1.0, 0.0, [14.0; 4]);
        if selected {
            let pulse = (v.now * 8.0).sin() * 0.5 + 0.5;
            list.push_outline(inset(r, -3.0), gold.with_alpha(0.5 + 0.4 * pulse), 2.0, 0.0, [17.0; 4]);
        }
        let fg = if selected { hex(theme::PANEL) } else { Color::WHITE.with_alpha(if can { 1.0 } else { 0.35 }) };
        let glyph = match a {
            Ability::Reroll => theme::ICON_REROLL,
            Ability::Hammer => theme::ICON_HAMMER,
            Ability::Bomb => theme::ICON_BOMB,
        };
        let ir = Rect::new(Point::new(r.origin.x + 8.0, r.origin.y), Size::new(28.0, r.height()));
        icon(list, glyph, ir, fg, 22.0);
        let tr = Rect::new(Point::new(r.origin.x + 38.0, r.origin.y + 6.0), Size::new(r.width() - 42.0, 18.0));
        text_nowrap(list, a.name(), tr, fg, 12.5, 800, TextAlign::LEFT | TextAlign::VCENTER);
        let cr = Rect::new(Point::new(r.origin.x + 38.0, r.origin.y + 23.0), Size::new(r.width() - 42.0, 16.0));
        let cost_col = if selected { hex(theme::PANEL) } else if can { gold } else { Color::WHITE.with_alpha(0.35) };
        text_nowrap(list, &format!("{} {}", a.cost(), theme::ICON_BOLT), cr, cost_col, 11.5, 800, TextAlign::LEFT | TextAlign::VCENTER);
    }
}

/// Кольцо уровня слева от меню: номер уровня и дуга прогресса до следующего.
fn level_ring(v: &View, game: &Game, list: &mut DisplayList) {
    let g = &v.geo;
    let pal = v.palette();
    let punch = anim::punch(clamp01((v.now - v.fx.level_born) / 0.7));
    let size = 40.0;
    let rect = Rect::new(Point::new(g.menu_btn.origin.x - size - 10.0, g.menu_btn.origin.y), Size::new(size, size));
    let (cx, cy) = (rect.origin.x + size * 0.5, rect.origin.y + size * 0.5);
    let rad = size * 0.5 - 2.5;
    list.push_rect(rect, Color::BLACK.with_alpha(0.16), [size * 0.5; 4]);
    arc(list, cx, cy, rad, 0.0, 1.0, Color::WHITE.with_alpha(0.14), 3.0);
    // дуга догоняет настоящий прогресс вместе с показанным счётом
    let shown = v.fx.score_shown.round() as i32;
    let p = if world::level_for(shown) < v.level { 1.0 } else { world::progress(shown.min(game.scoring.score)) };
    if p > 0.005 {
        arc(list, cx, cy, rad, 0.0, p, pal.accent.lerp(&Color::WHITE, punch), 3.0 + punch * 2.0);
    }
    if punch > 0.01 {
        let rr = rad + 4.0 + punch * 6.0;
        list.push_outline(Rect::new(Point::new(cx - rr, cy - rr), Size::new(rr * 2.0, rr * 2.0)), pal.accent.with_alpha(0.6 * punch), 2.0, 0.0, [rr; 4]);
    }
    text_nowrap(list, &v.level.to_string(), offset(rect, 0.0, -1.0), Color::WHITE, 17.0 * (1.0 + 0.3 * punch), 800, TextAlign::CENTER);
    let lr = Rect::new(Point::new(rect.origin.x - 10.0, rect.origin.y + size - 1.0), Size::new(size + 20.0, 13.0));
    text_nowrap(list, "УРОВЕНЬ", lr, hex(theme::TEXT_DIM).with_alpha(0.8), 8.5, 800, TextAlign::CENTER);
}

/// Дуга окружности от доли `a` до `b` (0 — сверху, по часовой).
fn arc(list: &mut DisplayList, cx: f32, cy: f32, r: f32, a: f32, b: f32, color: Color, width: f32) {
    let n = ((b - a) * 48.0).ceil().max(2.0) as usize;
    let pts: Vec<[f32; 2]> = (0..=n)
        .map(|i| {
            let u = (a + (b - a) * i as f32 / n as f32) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            [cx + r * u.cos(), cy + r * u.sin()]
        })
        .collect();
    list.push_line_strip(pts, color, width);
}

/// Плашка активного режима блоков на верхней кромке поля: образец кубика,
/// название и сколько ходов осталось.
fn skin_chip(v: &View, list: &mut DisplayList) {
    let Some(sk) = v.skin else { return };
    let a = sk.chip_alpha(v.now);
    if a <= 0.01 || v.fx.over_at.is_some() {
        return;
    }
    let g = &v.geo;
    let w = 214.0;
    let h = 26.0;
    let pop = anim::ease_out_back(clamp01((v.now - sk.born) / 0.45));
    let cx = g.board.origin.x + g.board.width() * 0.5;
    // между счётом и полем, не заходя на поле
    let y = g.board.origin.y - super::layout::GAP_BOARD + (super::layout::GAP_BOARD - h) * 0.5 - 2.0;
    let rect = scaled_about_center(Rect::new(Point::new(cx - w * 0.5, y), Size::new(w, h)), 0.6 + 0.4 * pop.min(1.1));
    let pal = v.palette();
    list.push_rect_bordered(rect, pal.panel.with_alpha(0.92 * a), [h * 0.5; 4], Border { width: 1.5, color: pal.accent.with_alpha(0.7 * a) });
    let look = world::look_for(v.variant, crate::game::Material::Plain, 3 + (v.now * 0.8) as u8 % 5, 0, Some(sk.material));
    let cube = Rect::new(Point::new(rect.origin.x + 7.0, rect.origin.y + (rect.height() - 16.0) * 0.5), Size::new(16.0, 16.0));
    let mut style = v.cube_style(&look, 7);
    style.blend = Some((pal.panel, 1.0 - a));
    draw_cube(list, cube, look.base, &style);
    let name = match look.finish {
        Some(f) => f.en.to_uppercase(),
        None => world::skin_name(sk.material).to_string(),
    };
    let label = format!("{} +{} · {} {}", name, sk.material.mode_bonus(), sk.moves_left, moves_word(sk.moves_left));
    let tr = Rect::new(Point::new(rect.origin.x + 26.0, rect.origin.y), Size::new(rect.width() - 30.0, rect.height()));
    text_nowrap(list, &label, tr, Color::WHITE.with_alpha(a), 12.5, 800, TextAlign::CENTER);
}

fn moves_word(n: u32) -> &'static str {
    match (n % 10, n % 100) {
        (1, x) if x != 11 => "ход",
        (2..=4, x) if !(12..=14).contains(&x) => "хода",
        _ => "ходов",
    }
}

/// Подсказка после проигрыша, пока итоги не открыты.
fn over_hint(v: &View, list: &mut DisplayList) {
    if !v.over_waiting() {
        return;
    }
    let g = &v.geo;
    let t0 = v.fx.over_at.unwrap_or(0.0) + OVER_DELAY;
    let a = clamp01((v.now - t0) / 0.4) * (0.75 + 0.25 * (v.now * 3.0).sin());
    // закрывает весь ряд способностей (после проигрыша они не нужны)
    let (l, r) = (g.abil_btns[0], g.abil_btns[2]);
    let rect = inset(Rect::new(l.origin, Size::new(r.origin.x + r.width() - l.origin.x, l.height())), -3.0);
    let pal = v.palette();
    list.push_rect_bordered(rect, pal.panel.with_alpha(0.94), [rect.height() * 0.5; 4], Border { width: 1.5, color: Color::WHITE.with_alpha(0.35 * a) });
    text_nowrap(list, "Коснитесь, чтобы увидеть итоги", rect, Color::WHITE.with_alpha(a), 15.0, 800, TextAlign::CENTER);
}

fn floats(v: &View, list: &mut DisplayList) {
    for f in &v.fx.floats {
        let t = f.progress(v.now);
        let k = anim::ease_out_cubic(t);
        let a = if t < 0.15 { t / 0.15 } else { 1.0 - clamp01((t - 0.55) / 0.45) };
        let pop = 1.0 + 0.35 * anim::punch(clamp01(t / 0.35));
        let y = f.pos.y - f.rise * k;
        let rect = Rect::new(Point::new(f.pos.x - 120.0, y - 24.0), Size::new(240.0, 48.0));
        text_shadowed(list, &f.text, rect, f.color.with_alpha(f.color.a * a), f.size * pop, f.weight, TextAlign::CENTER);
    }
}

fn banner(v: &View, list: &mut DisplayList) {
    let Some(b) = &v.fx.banner else { return };
    let t = clamp01((v.now - b.born) / BANNER_DUR);
    let appear = anim::ease_out_back(clamp01(t / 0.22));
    let fade = 1.0 - clamp01((t - 0.72) / 0.28);
    let a = appear.min(1.0) * fade;
    if a <= 0.0 {
        return;
    }
    let g = &v.geo;
    let cx = g.board.origin.x + g.board.width() * 0.5;
    let cy = g.board.origin.y + g.board.height() * 0.42 - t * 26.0;
    let size = 44.0 * appear;
    let rect = Rect::new(Point::new(cx - g.board.width() * 0.5, cy - 40.0), Size::new(g.board.width(), 80.0));
    // подложка-полоса
    list.push_gradient_rect(
        Rect::new(Point::new(rect.origin.x - 20.0, cy - 34.0), Size::new(rect.width() + 40.0, 68.0)),
        Gradient::Linear {
            angle_deg: 90.0,
            stops: vec![
                ColorStop::new(Color::from_srgb(0, 0, 0, 0.0), 0.0),
                ColorStop::new(Color::from_srgb(0, 0, 0, 0.42 * a), 0.5),
                ColorStop::new(Color::from_srgb(0, 0, 0, 0.0), 1.0),
            ],
        },
        [0.0; 4],
    );
    if b.color.g < 0.9 {
        // золотой баннер: мягкое сияние вокруг букв
        for (dx, dy) in [(-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0)] {
            text(list, &b.text, offset(rect, dx, dy), b.color.with_alpha(a * 0.28), size, 800, TextAlign::CENTER);
        }
    }
    text_shadowed(list, &b.text, rect, b.color.with_alpha(a), size, 800, TextAlign::CENTER);
    if let Some(sub) = &b.sub {
        let sr = Rect::new(Point::new(rect.origin.x, cy + 26.0), Size::new(rect.width(), 34.0));
        let (col, size, weight) = if v.variant.is_elvira() { (hex("#ffe9a8").with_alpha(a), 19.0 * appear, 700) } else { (hex(theme::GOLD).with_alpha(a), 22.0 * appear, 800) };
        text_shadowed(list, sub, sr, col, size, weight, TextAlign::CENTER);
    }
}

fn overlays(v: &View, game: &Game, list: &mut DisplayList) {
    let t = v.fx.overlay_t;
    if t <= 0.01 {
        return;
    }
    let kind = v.overlay().unwrap_or(if v.menu_open { Overlay::Menu } else { Overlay::Over });
    list.push_rect(v.geo.bounds, Color::from_srgb(6, 8, 30, 0.6 * t), [0.0; 4]);
    let dy = (1.0 - anim::ease_out_cubic(t)) * 48.0;
    match kind {
        Overlay::Menu => menu_card(v, game, list, &overlay::menu_card(&v.geo), t, dy),
        Overlay::Over => over_card(v, game, list, &overlay::over_card(&v.geo), t, dy),
    }
}

fn card_frame(list: &mut DisplayList, card: &Card, t: f32, dy: f32) -> Rect {
    let rect = offset(card.rect, 0.0, dy);
    list.push_shadow(rect, Color::from_srgb(0, 0, 0, 0.5 * t), 30.0, (0.0, 16.0), [22.0; 4]);
    list.push_gradient_rect(
        rect,
        Gradient::Linear {
            angle_deg: 180.0,
            stops: vec![ColorStop::new(hex(theme::CARD).lighten(0.08).with_alpha(t), 0.0), ColorStop::new(hex(theme::CARD).with_alpha(t), 1.0)],
        },
        [22.0; 4],
    );
    list.push_outline(inset(rect, 1.0), hex(theme::CARD_EDGE).with_alpha(0.8 * t), 1.5, 0.0, [22.0; 4]);
    rect
}

fn card_buttons(v: &View, list: &mut DisplayList, card: &Card, t: f32, dy: f32) {
    let stats = v.stats();
    for b in &card.buttons {
        let mut rect = offset(b.rect, 0.0, dy);
        let hot = b.rect.contains(v.pointer);
        let track_label;
        let (label, glyph): (&str, &str) = match b.action {
            overlay::Action::ToggleSound => (if stats.sound { "Звуки: вкл" } else { "Звуки: выкл" }, if stats.sound { theme::ICON_CHECK_ON } else { theme::ICON_CHECK_OFF }),
            overlay::Action::ToggleMusic => (if stats.music { "Музыка: вкл" } else { "Музыка: выкл" }, if stats.music { theme::ICON_CHECK_ON } else { theme::ICON_CHECK_OFF }),
            overlay::Action::ToggleVoice => (if stats.voice { "Голос: вкл" } else { "Голос: выкл" }, if stats.voice { theme::ICON_CHECK_ON } else { theme::ICON_CHECK_OFF }),
            overlay::Action::TrackNext if !b.label.is_empty() => {
                track_label = format!("{}/{}  {}", stats.track as usize + 1, crate::audio::track_count(), crate::audio::track_name(stats.track as usize));
                (track_label.as_str(), "")
            }
            _ => (b.label, b.icon),
        };
        let primary = match b.action {
            overlay::Action::Difficulty(i) => stats.difficulty as usize == i,
            overlay::Action::ToggleLoopAll => stats.loop_all,
            _ => b.primary,
        };
        let compact = rect.width() < 200.0;
        let narrow = rect.width() < 70.0;
        // три переключателя в ряд: короткие подписи без «вкл/выкл» (галочка и так видна)
        let label: &str = if compact && rect.width() < 130.0 { label.split(':').next().unwrap_or(label) } else { label };
        // Узкие кнопки-иконки — ровные круги: радиус 25 при ширине < 50
        // давал «лимончик» с острыми краями сверху и снизу.
        if narrow {
            let d = rect.width().min(rect.height());
            rect = Rect::new(
                Point::new(rect.origin.x + (rect.width() - d) * 0.5, rect.origin.y + (rect.height() - d) * 0.5),
                Size::new(d, d),
            );
        }
        let radius = [25.0f32.min(rect.width() * 0.5).min(rect.height() * 0.5); 4];
        let (bg, bg_hot) = if primary { (theme::BTN, theme::BTN_HOVER) } else { (theme::BTN_SECONDARY, theme::BTN_SECONDARY_HOVER) };
        let col = hex(if hot { bg_hot } else { bg }).with_alpha(t);
        if primary {
            list.push_shadow(rect, hex(theme::BTN).with_alpha(0.45 * t), 18.0, (0.0, 6.0), radius);
        }
        list.push_gradient_rect(
            rect,
            Gradient::Linear { angle_deg: 180.0, stops: vec![ColorStop::new(col.lighten(0.12), 0.0), ColorStop::new(col, 1.0)] },
            radius,
        );
        list.push_outline(inset(rect, 1.0), Color::WHITE.with_alpha(0.18 * t), 1.0, 0.0, radius);
        if narrow {
            let fg = Color::WHITE.with_alpha(t);
            match b.action {
                overlay::Action::TrackPrev => skip_icon(list, rect, fg, -1.0),
                overlay::Action::TrackNext => skip_icon(list, rect, fg, 1.0),
                overlay::Action::ToggleLoopAll => infinity_icon(list, rect, fg),
                _ => icon(list, glyph, rect, fg, 24.0),
            }
        } else if glyph.is_empty() {
            text_nowrap(list, label, rect, Color::WHITE.with_alpha(t), if compact { 14.0 } else { 15.0 }, 800, TextAlign::CENTER);
        } else {
            let ir = Rect::new(Point::new(rect.origin.x + if compact { 10.0 } else { 18.0 }, rect.origin.y), Size::new(28.0, rect.height()));
            icon(list, glyph, ir, Color::WHITE.with_alpha(t), if compact { 20.0 } else { 22.0 });
            if compact {
                let tr = Rect::new(Point::new(rect.origin.x + 40.0, rect.origin.y), Size::new(rect.width() - 46.0, rect.height()));
                text(list, label, tr, Color::WHITE.with_alpha(t), 14.5, 800, TextAlign::LEFT | TextAlign::VCENTER);
            } else {
                text(list, label, rect, Color::WHITE.with_alpha(t), 18.0, 800, TextAlign::CENTER);
            }
        }
    }
}

// Векторные иконки плеера: глифы Material Icons в мелком размере выходят
// кривыми (разная посадка по вертикали, «сплющенная» бесконечность).

/// «В начало / следующий»: шеврон + вертикальная черта; `dir` = −1 (назад) или +1.
fn skip_icon(list: &mut DisplayList, rect: Rect, color: Color, dir: f32) {
    let (cx, cy) = (rect.origin.x + rect.width() * 0.5, rect.origin.y + rect.height() * 0.5);
    let s = 7.0;
    let w = 3.0;
    let tip = cx + dir * s * 0.55;
    let back = tip - dir * s;
    list.push_line_strip(vec![[back, cy - s], [tip, cy], [back, cy + s]], color, w);
    let bar = tip + dir * 4.0;
    list.push_line_strip(vec![[bar, cy - s], [bar, cy + s]], color, w);
}

/// Бесконечность (лемниската Бернулли) — «все треки по кругу».
fn infinity_icon(list: &mut DisplayList, rect: Rect, color: Color) {
    let (cx, cy) = (rect.origin.x + rect.width() * 0.5, rect.origin.y + rect.height() * 0.5);
    let a = 11.0;
    const N: usize = 64;
    let pts: Vec<[f32; 2]> = (0..=N)
        .map(|i| {
            let t = i as f32 / N as f32 * std::f32::consts::TAU;
            let (sn, cs) = t.sin_cos();
            let d = 1.0 + sn * sn;
            [cx + a * cs / d, cy + a * 1.15 * sn * cs / d]
        })
        .collect();
    list.push_line_strip(pts, color, 2.4);
}

fn stat_row(list: &mut DisplayList, y: f32, x: f32, w: f32, label: &str, value: &str, t: f32) {
    let r = Rect::new(Point::new(x, y), Size::new(w, 26.0));
    text_nowrap(list, label, r, hex(theme::TEXT_DIM).with_alpha(t), 15.0, 600, TextAlign::LEFT | TextAlign::VCENTER);
    text_nowrap(list, value, r, Color::WHITE.with_alpha(t), 16.0, 800, TextAlign::RIGHT | TextAlign::VCENTER);
}

fn menu_card(v: &View, game: &Game, list: &mut DisplayList, card: &Card, t: f32, dy: f32) {
    card_frame(list, card, t, dy);
    let s = v.stats();
    let title = offset(card.title, 0.0, dy);
    text_shadowed(list, "ПАУЗА", title, Color::WHITE.with_alpha(t), 30.0, 800, TextAlign::CENTER);
    let body = offset(card.body, 0.0, dy);
    let x = body.origin.x;
    let w = body.width();
    let mut y = body.origin.y;
    let b = &s.best_by[v.variant.index()];
    stat_row(list, y, x, w, &format!("{}: рекорды Л · Н · С", v.variant.name()), &format!("{} · {} · {}", b[0], b[1], b[2]), t);
    y += 26.0;
    stat_row(list, y, x, w, "Сыграно партий", &s.games.to_string(), t);
    y += 26.0;
    stat_row(list, y, x, w, "Лучшее комбо · идеальных", &format!("×{} · {}", s.best_combo.max(game.scoring.best_combo), s.perfects), t);
    y += 28.0;
    let hint = Rect::new(Point::new(x, y), Size::new(w, 22.0));
    text_nowrap(list, "Сложность — для новой партии  ·  Q/W/E — способности", hint, hex(theme::TEXT_DIM).with_alpha(0.8 * t), 11.0, 500, TextAlign::CENTER);
    card_buttons(v, list, card, t, dy);
}

fn over_card(v: &View, game: &Game, list: &mut DisplayList, card: &Card, t: f32, dy: f32) {
    card_frame(list, card, t, dy);
    let s = v.stats();
    let new_best = game.scoring.score > 0 && game.scoring.score >= s.best && v.best_announced;
    let title = offset(card.title, 0.0, dy);
    if new_best {
        let pulse = 1.0 + 0.05 * (v.now * 6.0).sin();
        text_shadowed(list, "НОВЫЙ РЕКОРД!", title, hex(theme::GOLD).with_alpha(t), 28.0 * pulse, 800, TextAlign::CENTER);
    } else {
        text_shadowed(list, "ИГРА ОКОНЧЕНА", title, Color::WHITE.with_alpha(t), 28.0, 800, TextAlign::CENTER);
    }
    let body = offset(card.body, 0.0, dy);
    let x = body.origin.x;
    let w = body.width();
    let score_r = Rect::new(Point::new(x, body.origin.y), Size::new(w, 64.0));
    text_shadowed(list, &game.scoring.score.to_string(), score_r, Color::WHITE.with_alpha(t), 52.0, 800, TextAlign::CENTER);
    let tr = Rect::new(Point::new(x, body.origin.y + 62.0), Size::new(w, 24.0));
    let trophy = Rect::new(Point::new(x + w * 0.5 - 52.0, tr.origin.y), Size::new(24.0, 24.0));
    icon(list, theme::ICON_TROPHY, trophy, hex(theme::GOLD).with_alpha(t), 20.0);
    let best_r = Rect::new(Point::new(x + w * 0.5 - 24.0, tr.origin.y), Size::new(w * 0.5, 24.0));
    text(list, &format!("Рекорд  {}", s.best.max(game.scoring.score)), best_r, hex(theme::GOLD).with_alpha(t), 17.0, 800, TextAlign::LEFT | TextAlign::VCENTER);
    let mut y = body.origin.y + 96.0;
    stat_row(list, y, x, w, "Собрано линий", &game.scoring.lines.to_string(), t);
    y += 26.0;
    stat_row(list, y, x, w, "Лучшее комбо · Ходов", &format!("×{}  ·  {}", game.scoring.best_combo, game.scoring.moves), t);
    card_buttons(v, list, card, t, dy);
}
