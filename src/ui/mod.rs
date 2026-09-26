//! GameView — единственный элемент приложения: ввод (мышь, тач,
//! клавиатура), состояние эффектов и отрисовка через DisplayList.
//!
//! Модель партии живёт в `GameCtx::game` (RwSignal) — так её видят тесты и
//! другие экраны; элемент хранит только презентационное состояние.

pub mod cube;
mod demo;
pub mod fx;
pub mod layout;
pub mod overlay;
pub mod paint;
pub mod theme;

use std::any::Any;
use std::time::Duration;

use crate::anim::{self, FloatText, Particle, Shape as PShape};
use crate::audio::{self, Sfx};
use crate::game::{cell_color, cell_material, Ability, Difficulty, Game, Material, Placement, Rng, MAX_ENERGY, SIZE};
use crate::persist::{self, Stats};
use fx::*;
use layout::Geo;
use overlay::Action;
use syngui::prelude::*;
use syngui::input::CursorIcon;
use syngui::{LayoutHint, StyledElement};

/// Порог (px), после которого нажатие на фигуру считается перетаскиванием.
pub const DRAG_THRESHOLD: f32 = 3.0;
/// Радиус «магнитного» прилипания фигуры к клетке (в долях клетки).
pub const SNAP_RADIUS: f32 = 0.62;

#[derive(Clone, Copy)]
pub struct GameCtx {
    pub game: RwSignal<Game>,
    pub stats: RwSignal<Stats>,
}

impl GameCtx {
    pub fn new(seed: u64, stats: Stats) -> Self {
        GameCtx { game: use_signal(Game::new(seed)), stats: use_signal(stats) }
    }
}

/// Виджет-обёртка (без данных: всё берётся из контекста).
pub struct GameView;

impl Widget for GameView {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(View::new(use_context::<GameCtx>()))
    }
    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn mount(&self, _tree: &mut ElementTree, _parent_id: ElementId) {}
}

/// Перетаскивание фигуры из трея.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub slot: usize,
    pub start: Point,
    pub cur: Point,
    pub active: bool,
    pub touch: bool,
    pub touch_id: Option<u64>,
    /// Точка захвата относительно левого-верха рамки, в клетках.
    pub grab: (f32, f32),
    /// Подъём/увеличение фигуры (0 → 1).
    pub lift: f32,
}

/// Курсор клавиатурного режима.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KbCursor {
    pub slot: usize,
    pub r: i32,
    pub c: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    Menu,
    Over,
}

pub struct View {
    id: ElementId,
    bounds: Rect,
    classes: Vec<String>,
    dirty: DirtyFlags,
    pub ctx: GameCtx,
    pub geo: Geo,
    pub now: f32,
    pub pointer: Point,
    pub drag: Option<Drag>,
    pub kb: Option<KbCursor>,
    pub menu_open: bool,
    /// Выбранная способность, ждущая цели на поле.
    pub targeting: Option<Ability>,
    pub fx: Fx,
    pub best_announced: bool,
    /// Рекорд на момент начала партии — «NEW BEST!» показывается, когда побит он.
    pub best_at_start: i32,
    /// Демо-режим для скриншотов: ввод игнорируется.
    pub frozen: bool,
    /// Звук конца игры уже сыгран для текущей партии.
    over_sound_played: bool,
    /// Демо: отложенный ход `(время, слот, r, c)`.
    pub demo_move_at: Option<(f32, usize, i32, i32)>,
    pub prng: Rng,
    stats_dirty: bool,
    last_save: f32,
}

impl View {
    pub fn new(ctx: GameCtx) -> Self {
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(460.0, 820.0));
        let mut v = View {
            id: ElementId::new(),
            bounds,
            classes: Vec::new(),
            dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER,
            ctx,
            geo: layout::compute(bounds),
            now: 0.0,
            pointer: Point::new(-1.0, -1.0),
            drag: None,
            kb: None,
            menu_open: false,
            targeting: None,
            fx: Fx::new(),
            best_announced: false,
            best_at_start: {
                let st = ctx.stats.get_untracked();
                st.best_by[ctx.game.get_untracked().difficulty.index()]
            },
            frozen: false,
            over_sound_played: false,
            demo_move_at: None,
            prng: Rng::from_time(),
            stats_dirty: false,
            last_save: 0.0,
        };
        v.fx.tray_born = [0.0, 0.06, 0.12];
        let g = v.game();
        v.fx.score_shown = g.scoring.score as f32;
        if g.over {
            v.fx.over_at = Some(0.0);
        }
        demo::apply(&mut v);
        v
    }

    // ── доступ к модели ────────────────────────────────────────────────

    pub fn game(&self) -> Game {
        self.ctx.game.get_untracked()
    }

    pub fn stats(&self) -> Stats {
        self.ctx.stats.get_untracked()
    }

    /// Какой оверлей сейчас показан.
    pub fn overlay(&self) -> Option<Overlay> {
        if self.menu_open {
            return Some(Overlay::Menu);
        }
        match self.fx.over_at {
            Some(t) if self.now >= t + OVER_DELAY => Some(Overlay::Over),
            _ => None,
        }
    }

    /// Ввод по полю/трею заблокирован (оверлей или ожидание конца игры).
    fn input_locked(&self) -> bool {
        self.menu_open || self.fx.over_at.is_some()
    }

    pub fn input_locked_pub(&self) -> bool {
        self.input_locked()
    }

    // ── перетаскивание ──────────────────────────────────────────────────

    /// Левый-верх рамки и текущий размер клетки перетаскиваемой фигуры.
    pub fn drag_origin(&self, d: &Drag) -> (Point, f32) {
        let k = anim::ease_out_cubic(d.lift);
        let cell_now = anim::lerp(self.geo.tray_cell, self.geo.cell, k);
        let lift_px = self.geo.cell * if d.touch { 1.55 } else { 0.55 } * k;
        (
            Point::new(d.cur.x - d.grab.0 * cell_now, d.cur.y - d.grab.1 * cell_now - lift_px),
            cell_now,
        )
    }

    /// Куда прилипнет фигура слота `slot`, если её рамка стоит в `origin`.
    pub fn snap(&self, game: &Game, slot: usize, origin: Point) -> Option<(i32, i32)> {
        let piece = game.piece(slot)?;
        let (r0, c0) = self.geo.anchor_for_origin(origin);
        let mut best: Option<((i32, i32), f32)> = None;
        for dr in -1..=1 {
            for dc in -1..=1 {
                let (r, c) = (r0 + dr, c0 + dc);
                if !game.board.can_place(&piece.shape, r, c) {
                    continue;
                }
                let px = self.geo.grid.x + c as f32 * self.geo.cell;
                let py = self.geo.grid.y + r as f32 * self.geo.cell;
                let d = ((origin.x - px).powi(2) + (origin.y - py).powi(2)).sqrt() / self.geo.cell;
                if d <= SNAP_RADIUS && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                    best = Some(((r, c), d));
                }
            }
        }
        best.map(|(rc, _)| rc)
    }

    /// Текущая цель перетаскивания (валидный якорь) — для превью.
    pub fn drag_target(&self, game: &Game) -> Option<(usize, (i32, i32))> {
        let d = self.drag.as_ref()?;
        if !d.active {
            return None;
        }
        let (origin, _) = self.drag_origin(d);
        self.snap(game, d.slot, origin).map(|rc| (d.slot, rc))
    }

    fn begin_drag(&mut self, slot: usize, pos: Point, touch: bool, touch_id: Option<u64>) {
        let game = self.game();
        let Some(piece) = game.piece(slot) else { return };
        let o = self.geo.piece_origin_in_slot(slot, &piece.shape);
        let tc = self.geo.tray_cell;
        let grab = (
            ((pos.x - o.x) / tc).clamp(0.0, piece.shape.w as f32),
            ((pos.y - o.y) / tc).clamp(0.0, piece.shape.h as f32),
        );
        self.drag = Some(Drag { slot, start: pos, cur: pos, active: false, touch, touch_id, grab, lift: 0.0 });
        self.kb = None;
        audio::play(Sfx::Pick);
    }

    fn update_drag(&mut self, pos: Point) {
        if let Some(d) = self.drag {
            if d.active && self.now - self.fx.last_trail_at > 0.03 {
                let (origin, cell) = self.drag_origin(&d);
                self.fx.trail.push(TrailPoint { origin, cell, born: self.now });
                self.fx.last_trail_at = self.now;
                if self.fx.trail.len() > 8 {
                    self.fx.trail.remove(0);
                }
            }
        }
        if let Some(d) = self.drag.as_mut() {
            d.cur = pos;
            if !d.active {
                let dx = pos.x - d.start.x;
                let dy = pos.y - d.start.y;
                if dx * dx + dy * dy >= DRAG_THRESHOLD * DRAG_THRESHOLD {
                    d.active = true;
                }
            }
        }
    }

    fn end_drag(&mut self, pos: Point) -> bool {
        let Some(mut d) = self.drag.take() else { return false };
        d.cur = pos;
        if !d.active {
            return false;
        }
        let game = self.game();
        let (origin, cell_now) = self.drag_origin(&d);
        if let Some((r, c)) = self.snap(&game, d.slot, origin) {
            if self.commit(d.slot, r, c) {
                return true;
            }
        }
        self.fx.returning = Some(Return { slot: d.slot, from: origin, from_cell: cell_now, born: self.now });
        audio::play(Sfx::Back);
        false
    }

    fn cancel_drag(&mut self) {
        if let Some(d) = self.drag.take() {
            if d.active {
                let (origin, cell_now) = self.drag_origin(&d);
                self.fx.returning = Some(Return { slot: d.slot, from: origin, from_cell: cell_now, born: self.now });
                audio::play(Sfx::Back);
            }
        }
    }

    // ── ход ─────────────────────────────────────────────────────────────

    /// Поставить фигуру слота в `(r, c)`; при успехе запускает эффекты.
    pub fn commit(&mut self, slot: usize, r: i32, c: i32) -> bool {
        let mut game = self.game();
        let Some(pl) = game.place(slot, r, c) else { return false };
        self.ctx.game.set(game);
        self.on_placed(&pl);
        true
    }

    fn on_placed(&mut self, pl: &Placement) {
        let now = self.now;
        let game = self.game();
        self.kb = None;
        self.fx.last_place_at = now;
        audio::play(Sfx::Place);

        // центр поставленной фигуры в пикселях
        let (mut cx, mut cy) = (0.0, 0.0);
        for &(r, c) in &pl.cells {
            let cr = self.geo.cell_rect(r, c);
            cx += cr.origin.x + cr.width() * 0.5;
            cy += cr.origin.y + cr.height() * 0.5;
            self.fx.pops.push(Pop { r, c, born: now });
        }
        let n = pl.cells.len().max(1) as f32;
        cx /= n;
        cy /= n;

        // пыль при установке
        let base = theme::material_color(pl.piece.material, pl.piece.color);
        for &(r, c) in &pl.cells {
            let cr = self.geo.cell_rect(r, c);
            for _ in 0..2 {
                let ang = self.prng.range(0.0, std::f32::consts::TAU);
                let sp = self.prng.range(20.0, 70.0);
                self.fx.particles.push(Particle {
                    pos: Point::new(cr.origin.x + self.prng.range(0.2, 0.8) * cr.width(), cr.origin.y + self.prng.range(0.2, 0.8) * cr.height()),
                    vel: (ang.cos() * sp, ang.sin() * sp - 40.0),
                    size: self.prng.range(2.0, 4.0),
                    color: base.lighten(0.5).with_alpha(0.8),
                    life: self.prng.range(0.25, 0.45),
                    gravity: 120.0,
                    drag: 3.0,
                    round: true,
                    ..Default::default()
                });
            }
        }

        let lines = pl.clear.lines();
        if lines > 0 {
            let (ar, ac) = (pl.anchor.0 as f32 + pl.piece.shape.h as f32 * 0.5, pl.anchor.1 as f32 + pl.piece.shape.w as f32 * 0.5);
            for &(r, c, cell) in &pl.clear.cells {
                let dist = ((r as f32 + 0.5 - ar).powi(2) + (c as f32 + 0.5 - ac).powi(2)).sqrt();
                self.fx.vanish.push(Vanish { r, c, color: cell, born: now, delay: dist * VANISH_STAGGER, spawned: false });
            }
            self.fx.ripples.push(Ripple { origin: (ar, ac), born: now + 0.05 });
            // бонусы материалов: подпись и звук на каждый материал
            for (i, &(m, count, pts)) in pl.material_bonus.iter().enumerate() {
                self.fx.floats.push(FloatText {
                    text: format!("{} ×{}  +{}", m.name(), count, pts),
                    pos: Point::new(cx, cy + self.geo.cell * (0.6 + 0.55 * i as f32)),
                    born: now + 0.12 * (i as f32 + 1.0),
                    life: 1.3,
                    rise: 48.0,
                    size: 17.0,
                    color: theme::material_accent(m),
                    weight: 800,
                });
                let sfx = match m {
                    Material::Wood => Sfx::Wood,
                    Material::Crystal => Sfx::Crystal,
                    Material::Gold => Sfx::Gold,
                    Material::Lava => Sfx::Lava,
                    Material::Plain => continue,
                };
                audio::play(sfx);
            }
            for &r in &pl.clear.rows {
                self.fx.flashes.push(LineFlash { is_row: true, idx: r, born: now });
            }
            for &c in &pl.clear.cols {
                self.fx.flashes.push(LineFlash { is_row: false, idx: c, born: now });
            }
            if lines >= 2 || pl.perfect {
                self.fx.shake_born = now;
                self.fx.shake_mag = 2.0 + lines as f32 * 1.4 + if pl.perfect { 3.0 } else { 0.0 };
            }
            let combo = pl.gain.combo;
            let gold = theme::hex(theme::GOLD);
            // ударная волна от места очистки
            if lines >= 2 || combo >= 3 || pl.perfect {
                self.fx.shocks.push(Shockwave {
                    center: Point::new(cx, cy),
                    born: now,
                    color: if combo >= 3 || pl.perfect { gold } else { base.lighten(0.3) },
                    max_r: self.geo.board.width() * (0.55 + 0.15 * lines as f32).min(1.1),
                });
            }
            // вспышка экрана и конфетти на больших комбо
            if combo >= 3 || lines >= 3 {
                self.fx.screen_flashes.push(Flash { born: now, color: gold, strength: (0.07 + 0.025 * combo as f32).min(0.22) });
            }
            if pl.perfect {
                self.fx.screen_flashes.push(Flash { born: now, color: Color::WHITE, strength: 0.3 });
                self.confetti(90);
            } else if combo >= 5 || lines >= 4 {
                self.confetti(50);
            }
            let combo_text = if combo >= 2 { Some(format!("КОМБО ×{combo}")) } else { None };
            let (text, sub, color) = if pl.perfect {
                ("ИДЕАЛЬНО!".to_string(), combo_text, theme::hex(theme::GOLD))
            } else if let Some(p) = theme::praise(lines) {
                (p.to_string(), combo_text, if combo >= 3 { theme::hex(theme::GOLD) } else { Color::WHITE })
            } else if let Some(ct) = combo_text {
                (ct, None, theme::hex(theme::GOLD))
            } else {
                (String::new(), None, Color::WHITE)
            };
            if !text.is_empty() {
                self.fx.banner = Some(Banner { text, sub, born: now, color });
            }
            self.fx.score_punch_born = now;
            // «лестница» комбо: каждая очистка подряд звучит выше
            let semis = (combo.saturating_sub(1) as f32 * 1.5).min(9.0);
            audio::play_pitched(Sfx::clear(lines), semis);
            if combo >= 2 {
                audio::play(Sfx::Combo);
            }
            if pl.perfect {
                audio::play(Sfx::Perfect);
            }
        }

        // всплывающие очки
        let total = pl.gain.total();
        self.fx.floats.push(FloatText {
            text: format!("+{total}"),
            pos: Point::new(cx, cy - self.geo.cell * 0.3),
            born: now,
            life: if lines > 0 { 1.1 } else { 0.8 },
            rise: if lines > 0 { 64.0 } else { 40.0 },
            size: if lines > 0 { 26.0 + lines as f32 * 3.0 } else { 18.0 },
            color: if lines > 0 { theme::hex(theme::GOLD) } else { Color::WHITE.with_alpha(0.92) },
            weight: 800,
        });

        // рекорд (по сложности партии)
        let mut stats = self.stats();
        let di = game.difficulty.index();
        if game.scoring.score > stats.best_by[di] {
            stats.best_by[di] = game.scoring.score;
            stats.best = stats.best.max(game.scoring.score);
            self.ctx.stats.set(stats);
            self.stats_dirty = true;
            if !self.best_announced && self.best_at_start > 0 && game.scoring.score > self.best_at_start {
                self.best_announced = true;
                self.fx.best_flash_born = Some(now);
                audio::play(Sfx::Best);
                if self.fx.banner.is_none() {
                    self.fx.banner = Some(Banner { text: "НОВЫЙ РЕКОРД!".into(), sub: None, born: now, color: theme::hex(theme::GOLD) });
                }
            }
        }

        if pl.refilled {
            self.fx.tray_born = [now + 0.02, now + 0.09, now + 0.16];
            audio::play(Sfx::Refill);
        }
        if pl.over {
            let delay = if lines > 0 { VANISH_DUR + 0.45 } else { 0.2 };
            self.fx.over_at = Some(now + delay);
            self.finish_game(&game);
        }
    }

    /// Учёт завершённой (или брошенной) партии в статистике.
    fn finish_game(&mut self, game: &Game) {
        if game.scoring.moves == 0 {
            return;
        }
        let mut s = self.stats();
        s.games += 1;
        s.total_score += game.scoring.score as i64;
        s.total_lines += game.scoring.lines;
        s.best_combo = s.best_combo.max(game.scoring.best_combo);
        s.perfects += game.scoring.perfects;
        s.best = s.best.max(game.scoring.score);
        let di = game.difficulty.index();
        s.best_by[di] = s.best_by[di].max(game.scoring.score);
        self.ctx.stats.set(s);
        persist::save(&s);
        self.stats_dirty = false;
        self.last_save = self.now;
    }

    pub fn new_game(&mut self) {
        let old = self.game();
        if !old.over {
            self.finish_game(&old);
        }
        let seed = Rng::from_time().state() ^ self.prng.next_u64();
        let difficulty = Difficulty::from_index(self.stats().difficulty as usize);
        self.ctx.game.set(Game::with_difficulty(seed, difficulty));
        let particles = std::mem::take(&mut self.fx.particles);
        self.fx = Fx::new();
        self.fx.particles = particles;
        self.fx.tray_born = [self.now + 0.05, self.now + 0.12, self.now + 0.19];
        self.fx.score_shown = 0.0;
        self.best_announced = false;
        self.over_sound_played = false;
        self.best_at_start = self.stats().best_by[difficulty.index()];
        self.menu_open = false;
        self.targeting = None;
        audio::play(Sfx::Refill);
        self.kb = None;
        self.drag = None;
    }

    // ── клавиатура ──────────────────────────────────────────────────────

    fn kb_select(&mut self, slot: usize) {
        let game = self.game();
        let Some(p) = game.piece(slot) else { return };
        let (r, c) = match self.kb {
            Some(k) => (k.r, k.c),
            None => game.board.first_fit(&p.shape).unwrap_or((0, 0)),
        };
        let r = r.clamp(0, SIZE as i32 - p.shape.h as i32);
        let c = c.clamp(0, SIZE as i32 - p.shape.w as i32);
        self.kb = Some(KbCursor { slot, r, c });
    }

    fn kb_next_slot(&mut self) {
        let game = self.game();
        let start = self.kb.map(|k| k.slot + 1).unwrap_or(0);
        for i in 0..3 {
            let s = (start + i) % 3;
            if game.piece(s).is_some() {
                self.kb_select(s);
                return;
            }
        }
    }

    fn kb_move(&mut self, dr: i32, dc: i32) {
        if self.kb.is_none() {
            self.kb_next_slot();
            return;
        }
        let game = self.game();
        if let Some(mut k) = self.kb {
            if let Some(p) = game.piece(k.slot) {
                k.r = (k.r + dr).clamp(0, SIZE as i32 - p.shape.h as i32);
                k.c = (k.c + dc).clamp(0, SIZE as i32 - p.shape.w as i32);
                self.kb = Some(k);
            }
        }
    }

    fn kb_place(&mut self) {
        if let Some(k) = self.kb {
            if self.commit(k.slot, k.r, k.c) {
                // остаёмся в режиме клавиатуры на следующей фигуре
                let game = self.game();
                if !game.over {
                    for i in 0..3 {
                        let s = (k.slot + 1 + i) % 3;
                        if game.piece(s).is_some() {
                            self.kb_select(s);
                            break;
                        }
                    }
                }
            } else {
                self.fx.shake_born = self.now;
                self.fx.shake_mag = 1.5;
                audio::play(Sfx::Deny);
            }
        }
    }

    // ── способности ─────────────────────────────────────────────────────

    /// Нажатие на кнопку способности: обмен — сразу, молот/бомба — режим цели.
    fn press_ability(&mut self, a: Ability) {
        let game = self.game();
        if !game.can_use(a) {
            audio::play(Sfx::Deny);
            self.fx.shake_born = self.now;
            self.fx.shake_mag = 1.2;
            return;
        }
        if self.targeting == Some(a) {
            self.targeting = None;
            audio::play(Sfx::Click);
            return;
        }
        if a.needs_target() {
            self.targeting = Some(a);
            self.cancel_drag();
            self.kb = None;
            audio::play(Sfx::Pick);
        } else {
            self.apply_ability(a, None);
        }
    }

    fn apply_ability(&mut self, a: Ability, target: Option<(i32, i32)>) {
        let mut game = self.game();
        let Some(out) = game.use_ability(a, target) else {
            audio::play(Sfx::Deny);
            return;
        };
        self.ctx.game.set(game.clone());
        self.targeting = None;
        let now = self.now;
        let (mut cx, mut cy) = (0.0f32, 0.0f32);
        for (i, &(r, c, cell)) in out.removed.iter().enumerate() {
            let cr = self.geo.cell_rect(r, c);
            cx += cr.origin.x + cr.width() * 0.5;
            cy += cr.origin.y + cr.height() * 0.5;
            self.fx.vanish.push(Vanish { r, c, color: cell, born: now, delay: 0.02 * i as f32, spawned: false });
        }
        match a {
            Ability::Reroll => {
                self.fx.tray_born = [now + 0.02, now + 0.09, now + 0.16];
                audio::play(Sfx::Refill);
            }
            Ability::Hammer => {
                audio::play_pitched(Sfx::Place, -6.0);
                audio::play(Sfx::Wood);
                self.fx.shake_born = now;
                self.fx.shake_mag = 2.0;
            }
            Ability::Bomb => {
                let n = out.removed.len().max(1) as f32;
                let center = Point::new(cx / n, cy / n);
                self.fx.shocks.push(Shockwave { center, born: now, color: theme::hex("#ff7a2f"), max_r: self.geo.board.width() * 0.6 });
                self.fx.screen_flashes.push(Flash { born: now, color: theme::hex("#ff9a4a"), strength: 0.18 });
                self.fx.shake_born = now;
                self.fx.shake_mag = 5.0;
                if let Some((r, c)) = target {
                    self.fx.ripples.push(Ripple { origin: (r as f32 + 0.5, c as f32 + 0.5), born: now });
                }
                audio::play(Sfx::Lava);
                audio::play_pitched(Sfx::Clear3, -4.0);
            }
        }
        self.fx.floats.push(FloatText {
            text: format!("{} −{}", a.name(), a.cost()),
            pos: Point::new(self.geo.energy_bar.origin.x + self.geo.energy_bar.width() * 0.5, self.geo.energy_bar.origin.y - 6.0),
            born: now,
            life: 0.9,
            rise: 30.0,
            size: 15.0,
            color: theme::hex(theme::GOLD),
            weight: 800,
        });
        if game.over {
            self.fx.over_at = Some(now + 0.4);
            self.finish_game(&game);
        }
    }

    // ── оверлеи ─────────────────────────────────────────────────────────

    fn card_action_at(&self, pos: Point) -> Option<Action> {
        let card = match self.overlay()? {
            Overlay::Menu => overlay::menu_card(&self.geo),
            Overlay::Over => overlay::over_card(&self.geo),
        };
        card.buttons.iter().find(|b| b.rect.contains(pos)).map(|b| b.action)
    }

    fn apply(&mut self, a: Action) {
        audio::play(Sfx::Click);
        match a {
            Action::Resume => self.menu_open = false,
            Action::NewGame => self.new_game(),
            Action::Difficulty(i) => {
                let mut s = self.stats();
                s.difficulty = i.min(2) as u8;
                self.ctx.stats.set(s);
                persist::save(&s);
            }
            Action::ToggleLoopAll => {
                let mut s = self.stats();
                s.loop_all = !s.loop_all;
                self.ctx.stats.set(s);
                persist::save(&s);
                audio::set_loop_all(s.loop_all);
            }
            Action::TrackPrev | Action::TrackNext => {
                let mut s = self.stats();
                let n = audio::track_count() as i32;
                let cur = s.track as i32;
                let next = if a == Action::TrackPrev { (cur - 1 + n) % n } else { (cur + 1) % n };
                s.track = next as u8;
                if !s.music {
                    s.music = true;
                    audio::set_music(true);
                }
                self.ctx.stats.set(s);
                persist::save(&s);
                audio::set_track(next as usize);
            }
            Action::ToggleSound => {
                let mut s = self.stats();
                s.sound = !s.sound;
                self.ctx.stats.set(s);
                persist::save(&s);
                audio::set_sfx(s.sound);
                if s.sound {
                    audio::play(Sfx::Click);
                }
            }
            Action::ToggleMusic => {
                let mut s = self.stats();
                s.music = !s.music;
                self.ctx.stats.set(s);
                persist::save(&s);
                audio::set_music(s.music);
            }
        }
    }

    // ── события ─────────────────────────────────────────────────────────

    fn on_move(&mut self, pos: Point) -> EventResult {
        self.pointer = pos;
        if self.drag.is_some() {
            self.update_drag(pos);
            return EventResult::Handled;
        }
        EventResult::Ignored
    }

    fn on_press(&mut self, pos: Point, touch: bool, touch_id: Option<u64>) -> EventResult {
        self.pointer = pos;
        if let Some(_ov) = self.overlay() {
            if let Some(a) = self.card_action_at(pos) {
                self.apply(a);
            }
            return EventResult::Handled;
        }
        if self.input_locked() {
            return EventResult::Handled;
        }
        if self.geo.menu_btn.contains(pos) {
            self.menu_open = true;
            self.cancel_drag();
            self.targeting = None;
            audio::play(Sfx::Click);
            return EventResult::Handled;
        }
        if let Some(i) = self.geo.ability_at(pos) {
            self.press_ability(Ability::ALL[i]);
            return EventResult::Handled;
        }
        if let Some(a) = self.targeting {
            if let Some((r, c)) = self.geo.point_to_cell(pos) {
                self.apply_ability(a, Some((r, c)));
            } else {
                self.targeting = None;
                audio::play(Sfx::Click);
            }
            return EventResult::Handled;
        }
        if let Some(slot) = self.geo.slot_at(pos) {
            if self.game().piece(slot).is_some() {
                self.begin_drag(slot, pos, touch, touch_id);
                return EventResult::Handled;
            }
        }
        if self.kb.is_some() {
            self.kb = None;
            return EventResult::Handled;
        }
        EventResult::Ignored
    }

    fn on_release(&mut self, pos: Point) -> EventResult {
        self.pointer = pos;
        if self.drag.is_some() {
            self.end_drag(pos);
            return EventResult::Handled;
        }
        EventResult::Ignored
    }

    fn on_key(&mut self, key: Key) -> EventResult {
        match key {
            Key::Escape => {
                if self.drag.is_some() {
                    self.cancel_drag();
                } else if self.targeting.is_some() {
                    self.targeting = None;
                } else if self.kb.is_some() {
                    self.kb = None;
                } else if self.menu_open {
                    self.menu_open = false;
                    audio::play(Sfx::Click);
                } else if self.fx.over_at.is_none() {
                    self.menu_open = true;
                    audio::play(Sfx::Click);
                }
                EventResult::Handled
            }
            Key::N => {
                self.new_game();
                EventResult::Handled
            }
            Key::Enter | Key::Space => {
                if let Some(ov) = self.overlay() {
                    self.apply(match ov {
                        Overlay::Menu => Action::Resume,
                        Overlay::Over => Action::NewGame,
                    });
                } else if !self.input_locked() {
                    if let (Some(a), Some(k)) = (self.targeting, self.kb) {
                        self.apply_ability(a, Some((k.r, k.c)));
                    } else if self.kb.is_some() {
                        self.kb_place();
                    } else {
                        self.kb_next_slot();
                    }
                }
                EventResult::Handled
            }
            Key::Num1 | Key::Num2 | Key::Num3 if !self.input_locked() => {
                let slot = match key {
                    Key::Num1 => 0,
                    Key::Num2 => 1,
                    _ => 2,
                };
                self.cancel_drag();
                self.kb_select(slot);
                EventResult::Handled
            }
            Key::Tab if !self.input_locked() => {
                self.kb_next_slot();
                EventResult::Handled
            }
            Key::Q if !self.input_locked() => {
                self.press_ability(Ability::Reroll);
                EventResult::Handled
            }
            Key::W if !self.input_locked() => {
                self.press_ability(Ability::Hammer);
                EventResult::Handled
            }
            Key::E if !self.input_locked() => {
                self.press_ability(Ability::Bomb);
                EventResult::Handled
            }
            Key::Left if !self.input_locked() => {
                self.kb_move(0, -1);
                EventResult::Handled
            }
            Key::Right if !self.input_locked() => {
                self.kb_move(0, 1);
                EventResult::Handled
            }
            Key::Up if !self.input_locked() => {
                self.kb_move(-1, 0);
                EventResult::Handled
            }
            Key::Down if !self.input_locked() => {
                self.kb_move(1, 0);
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    // ── кадр ────────────────────────────────────────────────────────────

    fn tick(&mut self, dt: f32) {
        self.now += dt;
        let now = self.now;
        self.fx.prune(now);
        demo::tick(self);
        let game = self.game();

        // счёт догоняет
        let target = game.scoring.score as f32;
        self.fx.score_shown = anim::damp(self.fx.score_shown, target, 9.0, dt);
        if (target - self.fx.score_shown).abs() < 0.6 {
            self.fx.score_shown = target;
        }

        // подсветка фигур под курсором
        let hover_slot = if self.drag.is_none() && !self.input_locked() { self.geo.slot_at(self.pointer) } else { None };
        for i in 0..3 {
            let want = if hover_slot == Some(i) && game.piece(i).is_some() { 1.0 } else { 0.0 };
            self.fx.tray_hover[i] = anim::damp(self.fx.tray_hover[i], want, 14.0, dt);
        }

        if let Some(d) = self.drag.as_mut() {
            if d.active {
                d.lift = anim::damp(d.lift, 1.0, 14.0, dt);
                if d.lift > 0.995 {
                    d.lift = 1.0;
                }
            }
        }

        if self.overlay() == Some(Overlay::Over) && !self.over_sound_played {
            self.over_sound_played = true;
            audio::play(Sfx::Over);
        }
        let want_overlay = if self.overlay().is_some() { 1.0 } else { 0.0 };
        self.fx.overlay_t = anim::damp(self.fx.overlay_t, want_overlay, 11.0, dt);
        if (self.fx.overlay_t - want_overlay).abs() < 0.004 {
            self.fx.overlay_t = want_overlay;
        }

        // частицы исчезающих кубиков
        let mut bursts: Vec<(Rect, u8)> = Vec::new();
        for v in self.fx.vanish.iter_mut() {
            if !v.spawned && now >= v.born + v.delay {
                v.spawned = true;
                bursts.push((self.geo.cube_rect(v.r, v.c), v.color));
            }
        }
        for (rect, cell) in bursts {
            self.burst(rect, cell);
        }
        if game.scoring.combo >= 3 && !game.over && !self.menu_open {
            self.embers(dt);
        }
        self.fx.particles.update(dt);

        // режим «по кругу»: микшер сам перешёл к следующему треку
        let cur = audio::current_track() as u8;
        if cur != self.stats().track && self.stats().loop_all {
            let mut st = self.stats();
            st.track = cur;
            self.ctx.stats.set(st);
            self.stats_dirty = true;
        }
        if self.stats_dirty && now - self.last_save > 1.5 {
            persist::save(&self.stats());
            self.stats_dirty = false;
            self.last_save = now;
        }
    }

    /// Конфетти: цветные квадратики сыплются сверху поля.
    fn confetti(&mut self, n: usize) {
        let g = self.geo;
        for i in 0..n {
            let color = theme::cell_color(1 + (i % 7) as u8);
            let x = g.board.origin.x + self.prng.range(-0.05, 1.05) * g.board.width();
            let y = g.board.origin.y - self.prng.range(0.0, 60.0);
            self.fx.particles.push(Particle {
                pos: Point::new(x, y),
                vel: (self.prng.range(-60.0, 60.0), self.prng.range(40.0, 160.0)),
                size: self.prng.range(5.0, 10.0),
                color: color.lighten(self.prng.range(0.0, 0.3)).with_alpha(0.95),
                life: self.prng.range(1.6, 2.6),
                gravity: 140.0,
                drag: 1.2,
                round: i % 3 == 0,
                ..Default::default()
            });
        }
    }

    /// Искры «огненного» режима (комбо ≥ 3) — из пилюли комбо.
    fn embers(&mut self, dt: f32) {
        self.fx.ember_acc += dt * 26.0;
        let pill = self.geo.combo_pill();
        while self.fx.ember_acc >= 1.0 {
            self.fx.ember_acc -= 1.0;
            let x = pill.origin.x + self.prng.range(0.1, 0.9) * pill.width();
            let y = pill.origin.y + pill.height() * 0.5;
            let hot = self.prng.unit();
            let color = theme::hex(theme::GOLD).lerp(&theme::hex("#ff6a2a"), hot).with_alpha(0.9);
            self.fx.particles.push(Particle {
                pos: Point::new(x, y),
                vel: (self.prng.range(-18.0, 18.0), self.prng.range(-70.0, -30.0)),
                size: self.prng.range(2.0, 4.5),
                color,
                life: self.prng.range(0.5, 1.0),
                gravity: -40.0,
                drag: 1.5,
                round: true,
                ..Default::default()
            });
        }
    }

    fn burst(&mut self, rect: Rect, cell: u8) {
        let material = cell_material(cell);
        let base = theme::material_color(material, cell_color(cell));
        let cx = rect.origin.x + rect.width() * 0.5;
        let cy = rect.origin.y + rect.height() * 0.5;
        match material {
            Material::Wood => {
                for i in 0..7 {
                    let ang = self.prng.range(0.0, std::f32::consts::TAU);
                    let sp = self.prng.range(60.0, 220.0);
                    self.fx.particles.push(Particle {
                        pos: Point::new(cx, cy),
                        vel: (ang.cos() * sp, ang.sin() * sp - 80.0),
                        size: self.prng.range(3.0, 6.0),
                        color: base.lerp(&theme::hex("#e0b27a"), self.prng.unit() * 0.6).with_alpha(0.95),
                        life: self.prng.range(0.5, 0.9),
                        gravity: 520.0,
                        drag: 1.2,
                        shape: if i % 2 == 0 { PShape::BarH } else { PShape::BarV },
                        ..Default::default()
                    });
                }
                return;
            }
            Material::Crystal => {
                for i in 0..9 {
                    let ang = self.prng.range(0.0, std::f32::consts::TAU);
                    let sp = self.prng.range(80.0, 280.0);
                    self.fx.particles.push(Particle {
                        pos: Point::new(cx, cy),
                        vel: (ang.cos() * sp, ang.sin() * sp - 60.0),
                        size: self.prng.range(3.0, 7.0),
                        color: if i % 3 == 0 { Color::WHITE } else { theme::material_accent(Material::Crystal) }.with_alpha(0.95),
                        life: self.prng.range(0.5, 1.0),
                        gravity: 380.0,
                        drag: 1.4,
                        shape: if i % 3 == 0 { PShape::Spark } else { PShape::Square },
                        ..Default::default()
                    });
                }
                return;
            }
            Material::Gold => {
                for i in 0..8 {
                    let ang = self.prng.range(-2.6, -0.5);
                    let sp = self.prng.range(120.0, 300.0);
                    self.fx.particles.push(Particle {
                        pos: Point::new(cx, cy),
                        vel: (ang.cos() * sp, ang.sin() * sp),
                        size: self.prng.range(4.0, 7.0),
                        color: theme::material_accent(Material::Gold).lerp(&theme::hex(theme::GOLD), self.prng.unit()).with_alpha(0.98),
                        life: self.prng.range(0.7, 1.2),
                        gravity: 560.0,
                        drag: 0.8,
                        shape: if i % 4 == 0 { PShape::Spark } else { PShape::Round },
                        ..Default::default()
                    });
                }
                return;
            }
            Material::Lava => {
                for i in 0..10 {
                    let ang = self.prng.range(0.0, std::f32::consts::TAU);
                    let sp = self.prng.range(40.0, 200.0);
                    let hot = self.prng.unit();
                    self.fx.particles.push(Particle {
                        pos: Point::new(cx, cy),
                        vel: (ang.cos() * sp, ang.sin() * sp - 90.0),
                        size: self.prng.range(3.0, 8.0),
                        color: theme::hex("#ff5a1f").lerp(&theme::hex("#ffd166"), hot).with_alpha(0.95),
                        life: self.prng.range(0.5, 1.1),
                        gravity: -60.0,
                        drag: 1.8,
                        round: true,
                        shape: if i % 3 == 0 { PShape::Spark } else { PShape::Round },
                        ..Default::default()
                    });
                }
                return;
            }
            Material::Plain => {}
        }
        for i in 0..6 {
            let ang = self.prng.range(0.0, std::f32::consts::TAU);
            let sp = self.prng.range(70.0, 260.0);
            let color = match i % 3 {
                0 => base.lighten(0.55),
                1 => base.lighten(0.2),
                _ => Color::WHITE,
            };
            self.fx.particles.push(Particle {
                pos: Point::new(cx + self.prng.range(-0.3, 0.3) * rect.width(), cy + self.prng.range(-0.3, 0.3) * rect.height()),
                vel: (ang.cos() * sp, ang.sin() * sp - 60.0),
                size: self.prng.range(3.0, 7.5),
                color: color.with_alpha(0.95),
                life: self.prng.range(0.45, 0.95),
                gravity: 460.0,
                drag: 1.6,
                round: i % 2 == 0,
                ..Default::default()
            });
        }
    }
}

impl Element for View {
    fn update(&mut self, _widget: &dyn Widget, _ctx: &mut UpdateContext) {}

    fn layout(&mut self, constraints: Constraints) -> Size {
        let w = if constraints.max_width.is_finite() { constraints.max_width } else { 460.0 };
        let h = if constraints.max_height.is_finite() { constraints.max_height } else { 820.0 };
        self.bounds = Rect::new(self.bounds.origin, Size::new(w, h));
        self.geo = layout::compute(self.bounds);
        Size::new(w, h)
    }

    fn build_display_list(&self, list: &mut DisplayList, _clip: Rect) {
        paint::draw(self, list);
    }

    fn handle_event(&mut self, e: &Event, ctx: &mut EventContext) -> EventResult {
        if self.frozen {
            return EventResult::Ignored;
        }
        let r = match e {
            Event::MouseMove(p) => self.on_move(*p),
            Event::MouseDown { button: MouseButton::Left, position } => self.on_press(*position, false, None),
            Event::MouseUp { button: MouseButton::Left, position } => self.on_release(*position),
            Event::TouchStart { id, position } => {
                if self.drag.is_none() {
                    self.on_press(*position, true, Some(*id))
                } else {
                    EventResult::Ignored
                }
            }
            Event::TouchMove { id, position } => {
                if self.drag.map(|d| d.touch_id == Some(*id)).unwrap_or(false) {
                    self.on_move(*position)
                } else {
                    EventResult::Ignored
                }
            }
            Event::TouchEnd { id, position } => {
                if self.drag.map(|d| d.touch_id == Some(*id)).unwrap_or(false) {
                    self.on_release(*position)
                } else {
                    EventResult::Ignored
                }
            }
            Event::KeyDown(k) => self.on_key(*k),
            Event::BackPressed => {
                if self.menu_open {
                    self.menu_open = false;
                    EventResult::Handled
                } else {
                    EventResult::Ignored
                }
            }
            Event::AppSuspended => {
                audio::suspend();
                EventResult::Ignored
            }
            Event::AppResumed => {
                audio::resume();
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        };
        // курсор
        let hot = self.drag.is_some()
            || self.geo.menu_btn.contains(self.pointer)
            || (!self.input_locked() && self.geo.ability_at(self.pointer).is_some())
            || (!self.input_locked() && self.geo.slot_at(self.pointer).map(|s| self.game().piece(s).is_some()).unwrap_or(false))
            || self.card_action_at(self.pointer).is_some();
        ctx.set_cursor(if self.drag.map(|d| d.active).unwrap_or(false) {
            CursorIcon::Grabbing
        } else if self.targeting.is_some() && self.geo.point_to_cell(self.pointer).is_some() {
            CursorIcon::Crosshair
        } else if hot {
            CursorIcon::Pointer
        } else {
            CursorIcon::Default
        });
        if r.is_handled() {
            self.dirty |= DirtyFlags::RENDER;
        }
        r
    }

    fn animate(&mut self, dt: Duration) -> bool {
        self.tick(dt.as_secs_f32().min(0.1));
        self.dirty |= DirtyFlags::RENDER;
        true
    }

    fn needs_repaint(&self) -> bool {
        true
    }

    fn children(&self) -> &[ElementId] {
        &[]
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, p: Point) {
        self.bounds.origin = p;
        self.geo = layout::compute(self.bounds);
    }
    fn mark_dirty(&mut self, f: DirtyFlags) {
        self.dirty |= f;
    }
    fn clear_dirty(&mut self, f: DirtyFlags) {
        self.dirty.remove(f);
    }
    fn is_dirty(&self, f: DirtyFlags) -> bool {
        self.dirty.contains(f)
    }
    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }
    fn mount(&mut self, _tree: &mut ElementTree) {}
    fn element_type_name(&self) -> &str {
        "GameView"
    }
    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Center
    }
    fn set_classes(&mut self, c: Vec<String>) {
        self.classes = c;
    }
    fn get_classes(&self) -> &[String] {
        &self.classes
    }
    fn reset_mss_styles(&mut self) {}
    fn apply_computed_style(&mut self, _s: &ComputedStyle) {}
}

impl StyledElement for View {
    fn apply_style(&mut self, _s: &ComputedStyle) {}
    fn classes(&self) -> &[String] {
        &self.classes
    }
    fn set_classes(&mut self, c: Vec<String>) {
        self.classes = c;
    }
}
