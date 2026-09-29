//! GameView — единственный элемент приложения: ввод (мышь, тач,
//! клавиатура), состояние эффектов и отрисовка через DisplayList.
//!
//! Модель партии живёт в `GameCtx::game` (RwSignal) — так её видят тесты и
//! другие экраны; элемент хранит только презентационное состояние.

pub mod cube;
mod demo;
pub mod finish;
pub mod fx;
pub mod layout;
pub mod overlay;
pub mod paint;
pub mod start;
pub mod theme;
pub mod world;

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use crate::anim::{self, FloatText, Particle, Shape as PShape};
use crate::audio::{self, Sfx, Voice};
use crate::game::{cell_color, cell_finish, cell_material, Ability, Cell, Difficulty, Game, Material, Piece, Placement, Rng, SIZE};
use crate::lang;
use crate::persist::{self, Stats, Variant};
use finish::Burst;
use fx::*;
use layout::Geo;
use overlay::Action;
use start::StartAction;
use syngui::core::sync::Mutex;
use syngui::gpu::{ImageHandle, ImageLoadState, ImageSource, ImageStore};
use syngui::input::CursorIcon;
use syngui::prelude::*;
use syngui::render::TextureId;
use syngui::{LayoutHint, StyledElement};

/// Атлас спрайтов отделок (PNG 1536×1536, 8×8 тайлов), см. `tools/build_atlas.py`.
const ATLAS_BYTES: &[u8] = include_bytes!("../../assets/tex/atlas.png");
/// Длительность перехода между стартовой страницей и игрой.
pub const SCREEN_FADE: f32 = 0.45;

/// Порог (px), после которого нажатие на фигуру считается перетаскиванием.
pub const DRAG_THRESHOLD: f32 = 3.0;
/// Радиус «магнитного» прилипания фигуры к клетке (в долях клетки).
pub const SNAP_RADIUS: f32 = 0.62;

/// Экран приложения.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Start,
    Game,
}

#[derive(Clone, Copy)]
pub struct GameCtx {
    pub game: RwSignal<Game>,
    pub stats: RwSignal<Stats>,
    /// Экран при создании элемента (приложение — стартовая страница,
    /// тесты — сразу игра) и текущий экран.
    pub screen: RwSignal<Screen>,
    /// Вид игры (стартовая страница переключает).
    pub variant: RwSignal<Variant>,
}

impl GameCtx {
    pub fn new(seed: u64, stats: Stats) -> Self {
        GameCtx { game: use_signal(Game::new(seed)), stats: use_signal(stats), screen: use_signal(Screen::Game), variant: use_signal(Variant::Standard) }
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
    /// Итоги партии показаны. Без касания они не появляются: после
    /// проигрыша поле и фигуры остаются на виду — видно, почему ходов нет.
    pub over_card: bool,
    over_card_at: f32,
    /// Демо: отложенный ход `(время, слот, r, c)`.
    pub demo_move_at: Option<(f32, usize, i32, i32)>,
    pub prng: Rng,
    stats_dirty: bool,
    last_save: f32,
    /// Партия изменилась — записать на диск в ближайшем кадре.
    game_dirty: bool,
    /// Тап уже обработан на TouchStart: syngui на отпускании пальца ещё
    /// синтезирует MouseDown/MouseUp — их надо проглотить, иначе каждое
    /// нажатие срабатывает дважды.
    touch_pressed: bool,
    /// Последний трек, о котором сообщил микшер (см. режим «по кругу» в tick).
    seen_track: usize,
    /// Уровень партии (из очков) и переход между мирами фона.
    pub level: u32,
    pub world: world::WorldFade,
    /// Режим блоков («всё деревянное»…) и предыдущий — для волны смены.
    pub skin: Option<world::SkinMode>,
    pub skin_prev: Option<world::SkinMode>,
    /// Вид игры и экран.
    pub variant: Variant,
    pub screen: Screen,
    /// Момент смены экрана (переход-затухание).
    pub screen_born: f32,
    /// Нажатая карточка стартовой страницы (индекс, момент).
    pub start_press: Option<(usize, f32)>,
    /// Есть ли сохранённая партия у вида игры (подпись «Продолжить»).
    pub start_saved: [bool; 2],
    /// Хранилище картинок и атлас отделок (текстура готова — `atlas`).
    image_store: Option<Arc<Mutex<ImageStore>>>,
    atlas_handle: Option<ImageHandle>,
    pub atlas: Option<TextureId>,
    /// Последняя реплика и момент последней (чтобы не тараторить).
    last_praise: Option<lang::Word>,
    last_voice_at: f32,
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
                st.best_of(ctx.variant.get_untracked(), ctx.game.get_untracked().difficulty)
            },
            frozen: false,
            over_sound_played: false,
            over_card: false,
            over_card_at: -10.0,
            demo_move_at: None,
            prng: Rng::from_time(),
            stats_dirty: false,
            last_save: 0.0,
            game_dirty: false,
            touch_pressed: false,
            seen_track: audio::current_track(),
            level: 1,
            world: world::WorldFade { from: 0, to: 0, born: -100.0 },
            skin: None,
            skin_prev: None,
            variant: ctx.variant.get_untracked(),
            screen: ctx.screen.get_untracked(),
            screen_born: -100.0,
            start_press: None,
            start_saved: [false; 2],
            image_store: None,
            atlas_handle: None,
            atlas: None,
            last_praise: None,
            last_voice_at: -100.0,
        };
        v.fx.tray_born = [0.0, 0.06, 0.12];
        match v.screen {
            Screen::Game => v.sync_from_game(),
            Screen::Start => v.refresh_saved(),
        }
        demo::apply(&mut v);
        v
    }

    /// Презентационное состояние по текущей партии (после загрузки/входа):
    /// показанный счёт, итоги, уровень, мир, режим блоков.
    fn sync_from_game(&mut self) {
        let g = self.game();
        self.fx.score_shown = g.scoring.score as f32;
        self.best_at_start = self.stats().best_of(self.variant, g.difficulty);
        self.best_announced = false;
        self.over_sound_played = g.over;
        if g.over {
            self.fx.over_at = Some(self.now);
            self.over_card = true;
        } else {
            self.fx.over_at = None;
            self.over_card = false;
        }
        self.level = world::level_for(g.scoring.score);
        let wi = world::world_index_for(self.level, self.variant);
        self.world = world::WorldFade { from: wi, to: wi, born: -100.0 };
        self.skin = None;
        self.skin_prev = None;
        if let Some(m) = g.mode.filter(|_| !g.over && g.mode_moves > 0) {
            let mut s = world::SkinMode::new(m, -100.0);
            s.moves_left = g.mode_moves;
            self.skin = Some(s);
        }
    }

    fn refresh_saved(&mut self) {
        if self.frozen {
            return;
        }
        for (i, v) in Variant::ALL.iter().enumerate() {
            self.start_saved[i] = persist::load_game(*v).map(|g| !g.over).unwrap_or(false);
        }
    }

    // ── экраны ──────────────────────────────────────────────────────────

    /// Войти в игру выбранного вида: загрузить его партию (или начать новую).
    pub fn enter_game(&mut self, variant: Variant) {
        let now = self.now;
        self.variant = variant;
        self.ctx.variant.set(variant);
        let mut st = self.stats();
        if st.variant != variant.index() as u8 {
            st.variant = variant.index() as u8;
            self.ctx.stats.set(st);
            persist::save(&st);
        }
        let game = if self.frozen { Some(self.game()) } else { persist::load_game(variant) };
        let game = game.unwrap_or_else(|| {
            let seed = Rng::from_time().state() ^ self.prng.next_u64();
            Game::with_difficulty(seed, Difficulty::from_index(st.difficulty as usize))
        });
        self.ctx.game.set(game);
        let particles = std::mem::take(&mut self.fx.particles);
        self.fx = Fx::new();
        self.fx.particles = particles;
        self.fx.tray_born = [now + 0.25, now + 0.32, now + 0.39];
        self.menu_open = false;
        self.targeting = None;
        self.kb = None;
        self.drag = None;
        self.sync_from_game();
        self.screen = Screen::Game;
        self.screen_born = now;
        self.ctx.screen.set(Screen::Game);
        self.game_dirty = true;
        if variant.is_deluxe() && !self.game().over {
            self.fx.banner_queue.push(Banner { text: lang::WELCOME.en.to_uppercase(), sub: Some(lang::WELCOME.ru.to_string()), born: now, color: theme::hex(theme::GOLD) });
            self.say(Voice::Welcome, now);
        }
    }

    /// На стартовую страницу; партия сохраняется.
    pub fn go_home(&mut self) {
        if self.screen == Screen::Start {
            return;
        }
        if !self.frozen {
            persist::save_game(self.variant, &self.game());
            persist::save(&self.stats());
        }
        self.cancel_drag();
        self.menu_open = false;
        self.targeting = None;
        self.kb = None;
        self.screen = Screen::Start;
        self.screen_born = self.now;
        self.ctx.screen.set(Screen::Start);
        self.refresh_saved();
        audio::play(Sfx::Chime);
    }

    /// Доля перехода между экранами (0 — только что сменился, 1 — завершён).
    pub fn screen_t(&self) -> f32 {
        anim::clamp01((self.now - self.screen_born) / SCREEN_FADE)
    }

    fn on_start_action(&mut self, a: StartAction) {
        match a {
            StartAction::Play(v) => {
                let i = v.index();
                self.start_press = Some((i, self.now));
                audio::play(Sfx::Chime);
                self.enter_game(v);
            }
            StartAction::ToggleSound => self.apply(Action::ToggleSound),
            StartAction::ToggleMusic => self.apply(Action::ToggleMusic),
            StartAction::ToggleVoice => self.apply(Action::ToggleVoice),
        }
    }

    /// Реплика (только «Люкс»): не чаще раза в 1.2 с, кроме важных.
    fn say(&mut self, v: Voice, now: f32) {
        if !self.variant.is_deluxe() || !self.stats().voice {
            return;
        }
        if v.rank() < 5 && now - self.last_voice_at < 1.2 {
            return;
        }
        self.last_voice_at = now;
        audio::say(v);
    }

    // ── доступ к модели ────────────────────────────────────────────────

    /// Палитра мира на текущий кадр.
    pub fn palette(&self) -> world::Palette {
        self.world.palette(self.now)
    }

    /// Материал режима для кубика с задержкой волны `delay` (если режим действует).
    pub fn skin_for(&self, delay: f32) -> Option<Material> {
        [self.skin, self.skin_prev]
            .into_iter()
            .flatten()
            .find(|s| s.on(self.now, delay))
            .map(|s| s.material)
    }

    /// Вспышка волны смены режима для кубика с задержкой `delay`.
    pub fn skin_flash(&self, delay: f32) -> f32 {
        [self.skin, self.skin_prev].into_iter().flatten().map(|s| s.flash(self.now, delay)).fold(0.0, f32::max)
    }

    /// Вид кубика поля `(r, c)` со значением клетки `val`.
    pub fn cell_look(&self, val: Cell, r: usize, c: usize) -> world::Look {
        world::look_for(self.variant, cell_material(val), cell_color(val), cell_finish(val), self.skin_for(world::cell_delay(r, c)))
    }

    /// Вид фигуры в трее / в руке.
    pub fn piece_look(&self, p: &Piece) -> world::Look {
        world::look_for(self.variant, p.material, p.color, p.finish, self.skin_for(world::TRAY_DELAY))
    }

    /// Стиль кубика по виду (текстура атласа — если загружена).
    pub fn cube_style(&self, look: &world::Look, seed: u32) -> cube::CubeStyle {
        cube::CubeStyle::default().with_look(look, seed, self.now).with_texture(self.atlas)
    }

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
            Some(t) if self.over_card && self.now >= t + OVER_DELAY => Some(Overlay::Over),
            _ => None,
        }
    }

    /// Партия окончена, итоги ещё не открыты (поле на виду).
    pub fn over_waiting(&self) -> bool {
        !self.over_card && self.fx.over_at.map(|t| self.now >= t + OVER_DELAY).unwrap_or(false)
    }

    fn show_over_card(&mut self) {
        self.over_card = true;
        self.over_card_at = self.now;
        audio::play(Sfx::Click);
    }

    /// Ввод по полю/трею заблокирован (оверлей, ожидание конца игры,
    /// стартовая страница).
    fn input_locked(&self) -> bool {
        self.menu_open || self.fx.over_at.is_some() || self.screen != Screen::Game
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
        self.game_dirty = true;
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
            self.fx.lands.push(Land { r, c, born: now });
        }
        let n = pl.cells.len().max(1) as f32;
        cx /= n;
        cy /= n;

        // пыль при установке
        let base = self.piece_look(&pl.piece).base;
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
            // («Люкс» — английское имя отделки с переводом)
            let elv = self.variant.is_deluxe();
            for (i, &(m, count, pts)) in pl.material_bonus.iter().enumerate() {
                let name = if elv {
                    let f = finish::get(m.finish());
                    format!("{} · {}", f.en.to_uppercase(), f.ru)
                } else {
                    m.name().to_string()
                };
                self.fx.floats.push(FloatText {
                    text: format!("{} ×{}  +{}", name, count, pts),
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
            if elv {
                // звук отделок очищенных обычных кубиков (не больше двух разных)
                let mut played: Vec<Sfx> = Vec::new();
                for &(_, _, cell) in &pl.clear.cells {
                    if cell_material(cell) != Material::Plain {
                        continue;
                    }
                    let f = finish::get(finish::resolve(cell_finish(cell), Material::Plain, cell_color(cell)));
                    if !played.contains(&f.sfx) && played.len() < 2 {
                        played.push(f.sfx);
                        audio::play_pitched(f.sfx, self.prng.range(-1.0, 1.0));
                    }
                }
            }
            if let Some((m, count, pts)) = pl.mode_bonus {
                let i = pl.material_bonus.len();
                let name = if elv {
                    let f = finish::get(m.finish());
                    format!("{} · {}", f.en.to_uppercase(), f.ru)
                } else {
                    world::skin_name(m).to_string()
                };
                self.fx.floats.push(FloatText {
                    text: format!("{} ×{}  +{}", name, count, pts),
                    pos: Point::new(cx, cy + self.geo.cell * (0.6 + 0.55 * i as f32)),
                    born: now + 0.12 * (i as f32 + 1.0),
                    life: 1.3,
                    rise: 48.0,
                    size: 17.0,
                    color: theme::material_accent(m),
                    weight: 800,
                });
            }
            for &r in &pl.clear.rows {
                self.fx.flashes.push(LineFlash { is_row: true, idx: r, born: now });
                self.fx.beams.push(Beam { is_row: true, idx: r, born: now, from: ac, color: base.lerp(&Color::WHITE, 0.6) });
                self.line_sparkles(true, r, base);
            }
            for &c in &pl.clear.cols {
                self.fx.flashes.push(LineFlash { is_row: false, idx: c, born: now });
                self.fx.beams.push(Beam { is_row: false, idx: c, born: now, from: ar, color: base.lerp(&Color::WHITE, 0.6) });
                self.line_sparkles(false, c, base);
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
            let (text, sub, color) = if elv {
                // «Люкс»: английская реплика крупно, перевод и комбо ниже, голос
                let word = lang::praise(&mut self.prng, lines, combo, pl.perfect, self.last_praise);
                match word {
                    Some(w) => {
                        self.last_praise = Some(w);
                        if let Some(v) = w.voice {
                            self.say(v, now + 0.05);
                        }
                        let sub = if combo >= 2 { format!("{} · {} ×{combo}", w.ru, lang::COMBO.en.to_lowercase()) } else { w.ru.to_string() };
                        let gold = combo >= 3 || lines >= 3 || pl.perfect;
                        (w.en.to_uppercase(), Some(sub), if gold { theme::hex(theme::GOLD) } else { Color::WHITE })
                    }
                    None => (String::new(), None, Color::WHITE),
                }
            } else {
                let combo_text = if combo >= 2 { Some(format!("КОМБО ×{combo}")) } else { None };
                if pl.perfect {
                    ("ИДЕАЛЬНО!".to_string(), combo_text, theme::hex(theme::GOLD))
                } else if let Some(p) = theme::praise(lines) {
                    (p.to_string(), combo_text, if combo >= 3 { theme::hex(theme::GOLD) } else { Color::WHITE })
                } else if let Some(ct) = combo_text {
                    (ct, None, theme::hex(theme::GOLD))
                } else {
                    (String::new(), None, Color::WHITE)
                }
            };
            if !text.is_empty() {
                self.fx.banner = Some(Banner { text, sub, born: now, color });
            }
            if elv && (combo >= 3 || lines >= 3) {
                // блик-волна по всему полю
                self.fx.ripples.push(Ripple { origin: (ar, ac), born: now + 0.25 });
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

        // рекорд (по виду игры и сложности партии)
        let mut stats = self.stats();
        let (vi, di) = (self.variant.index(), game.difficulty.index());
        if game.scoring.score > stats.best_by[vi][di] {
            stats.best_by[vi][di] = game.scoring.score;
            stats.best = stats.best.max(game.scoring.score);
            self.ctx.stats.set(stats);
            self.stats_dirty = true;
            if !self.best_announced && self.best_at_start > 0 && game.scoring.score > self.best_at_start {
                self.best_announced = true;
                self.fx.best_flash_born = Some(now);
                audio::play(Sfx::Best);
                self.say(Voice::NewRecord, now);
                if self.fx.banner.is_none() {
                    let (text, sub) = if self.variant.is_deluxe() { (lang::NEW_RECORD.en.to_uppercase(), Some(lang::NEW_RECORD.ru.to_string())) } else { ("НОВЫЙ РЕКОРД!".to_string(), None) };
                    self.fx.banner = Some(Banner { text, sub, born: now, color: theme::hex(theme::GOLD) });
                }
            }
        }

        // уровень: мир фона меняется, фейерверк, новый режим блоков
        let lvl = world::level_for(game.scoring.score);
        if lvl > self.level && !pl.over {
            self.level_up(lvl);
        } else if !pl.over {
            self.skin_after_move();
        }

        if pl.refilled {
            self.fx.tray_born = [now + 0.02, now + 0.09, now + 0.16];
            audio::play(Sfx::Refill);
        }
        if pl.over {
            let (text, sub) = if self.variant.is_deluxe() { (lang::NO_MOVES.en.to_uppercase(), Some(lang::NO_MOVES.ru.to_string())) } else { ("НЕТ ХОДОВ".to_string(), None) };
            self.fx.banner_queue.push(Banner { text, sub, born: now, color: theme::hex(theme::DANGER).lerp(&Color::WHITE, 0.25) });
            let delay = if lines > 0 { VANISH_DUR + 0.45 } else { 0.2 };
            self.fx.over_at = Some(now + delay);
            self.finish_game(&game);
        }
    }

    // ── уровни и режимы ─────────────────────────────────────────────────

    /// Плавно перейти к миру `idx`.
    fn set_world(&mut self, idx: usize) {
        let now = self.now;
        let cur = if self.world.t(now) >= 0.5 { self.world.to } else { self.world.from };
        if cur == idx && self.world.to == idx {
            return;
        }
        self.world = world::WorldFade { from: cur, to: idx, born: now };
    }

    fn level_up(&mut self, level: u32) {
        let now = self.now;
        self.level = level;
        self.fx.level_born = now;
        let wi = world::world_index_for(level, self.variant);
        self.set_world(wi);
        let w = &world::WORLDS[wi];
        let accent = theme::hex(w.accent);
        let title = if self.variant.is_deluxe() { format!("{} {level}", lang::LEVEL.en.to_uppercase()) } else { format!("УРОВЕНЬ {level}") };
        self.fx.banner_queue.push(Banner { text: title, sub: Some(w.name.to_uppercase()), born: now, color: Color::WHITE });
        self.fx.screen_flashes.push(Flash { born: now + 0.1, color: accent, strength: 0.16 });
        // фейерверк: залпы над полем
        let b = self.geo.board;
        for i in 0..7 {
            let pos = Point::new(
                b.origin.x + b.width() * self.prng.range(0.1, 0.9),
                b.origin.y + b.height() * self.prng.range(0.05, 0.6),
            );
            let color = if i % 2 == 0 { accent } else { theme::cell_color(1 + self.prng.index(7) as u8) };
            self.fx.fireworks.push(Firework { at: now + 0.25 + 0.17 * i as f32 + self.prng.range(0.0, 0.08), pos, color });
        }
        audio::play(if self.variant.is_deluxe() { Sfx::Harp } else { Sfx::Perfect });
        self.start_skin(now + 0.5);
    }

    /// Включить случайный режим блоков (не повторяя текущий).
    fn start_skin(&mut self, at: f32) {
        let cur = self.skin.map(|s| s.material);
        let choices: Vec<Material> = world::SKINS.iter().copied().filter(|m| Some(*m) != cur).collect();
        let m = choices[self.prng.index(choices.len())];
        if let Some(mut old) = self.skin.take() {
            if old.end_at.is_none() {
                old.end_at = Some(at);
            }
            self.skin_prev = Some(old);
        }
        self.skin = Some(world::SkinMode::new(m, at));
        self.set_game_mode(Some(m));
        let sfx = match m {
            Material::Wood => Sfx::Wood,
            Material::Crystal => Sfx::Crystal,
            Material::Gold => Sfx::Gold,
            _ => Sfx::Lava,
        };
        audio::play(sfx);
    }

    /// Режим блоков в модели партии — от него зависит бонус очков.
    fn set_game_mode(&mut self, m: Option<Material>) {
        let mut g = self.game();
        let moves = self.skin.filter(|_| m.is_some()).map(|s| s.moves_left).unwrap_or(0);
        if g.mode != m || g.mode_moves != moves {
            g.mode = m;
            g.mode_moves = moves;
            self.ctx.game.set(g);
            self.game_dirty = true;
        }
    }

    /// Ход сделан: режим блоков тикает, иногда режим включается сам.
    fn skin_after_move(&mut self) {
        let now = self.now;
        let moves = self.game().scoring.moves;
        let roll = self.prng.below(100);
        let mut ended = false;
        match self.skin.as_mut() {
            Some(s) if s.end_at.is_none() => {
                s.moves_left = s.moves_left.saturating_sub(1);
                if s.moves_left == 0 {
                    s.end_at = Some(now + 0.3);
                    ended = true;
                }
            }
            None if moves > 6 && roll < 4 => {
                self.start_skin(now + 0.3);
                if let Some(s) = self.skin {
                    self.fx.banner_queue.push(Banner { text: format!("РЕЖИМ: {}", world::skin_name(s.material)), sub: Some(format!("{} ходов", world::SKIN_MOVES)), born: now, color: Color::WHITE });
                }
            }
            _ => {}
        }
        if ended {
            self.set_game_mode(None);
        } else if let Some(s) = self.skin.filter(|s| s.end_at.is_none()) {
            self.set_game_mode(Some(s.material));
        }
    }

    fn launch_firework(&mut self, fw: Firework) {
        let n = 30;
        for i in 0..n {
            let ang = i as f32 / n as f32 * std::f32::consts::TAU + self.prng.range(-0.1, 0.1);
            let sp = self.prng.range(120.0, 240.0);
            let white = i % 5 == 0;
            self.fx.particles.push(Particle {
                pos: fw.pos,
                vel: (ang.cos() * sp, ang.sin() * sp),
                size: self.prng.range(3.0, 5.5),
                color: if white { Color::WHITE } else { fw.color.lighten(self.prng.range(0.0, 0.3)) }.with_alpha(0.95),
                life: self.prng.range(0.8, 1.3),
                gravity: 110.0,
                drag: 2.2,
                shape: if i % 3 == 0 { PShape::Spark } else { PShape::Round },
                ..Default::default()
            });
        }
        self.fx.shocks.push(Shockwave { center: fw.pos, born: self.now, color: fw.color, max_r: 46.0 });
        audio::play_pitched(Sfx::Pick, self.prng.range(-3.0, 5.0));
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
        let (vi, di) = (self.variant.index(), game.difficulty.index());
        s.best_by[vi][di] = s.best_by[vi][di].max(game.scoring.score);
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
        self.game_dirty = true;
        let particles = std::mem::take(&mut self.fx.particles);
        self.fx = Fx::new();
        self.fx.particles = particles;
        self.fx.tray_born = [self.now + 0.05, self.now + 0.12, self.now + 0.19];
        self.fx.score_shown = 0.0;
        self.best_announced = false;
        self.over_sound_played = false;
        self.over_card = false;
        self.best_at_start = self.stats().best_of(self.variant, difficulty);
        self.menu_open = false;
        self.targeting = None;
        self.level = 1;
        self.set_world(world::world_index_for(1, self.variant));
        if let Some(mut s) = self.skin.take() {
            s.end_at.get_or_insert(self.now);
            self.skin_prev = Some(s);
        }
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
        self.game_dirty = true;
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
            Action::ViewBoard => self.over_card = false,
            Action::NewGame => self.new_game(),
            Action::Home => self.go_home(),
            Action::ToggleVoice => {
                let mut s = self.stats();
                s.voice = !s.voice;
                self.ctx.stats.set(s);
                persist::save(&s);
                audio::set_voice(s.voice);
                if s.voice {
                    self.last_voice_at = -100.0;
                    audio::say(Voice::Nice);
                }
            }
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
        if self.screen == Screen::Start {
            if self.screen_t() < 0.6 {
                return EventResult::Handled;
            }
            if let Some(a) = start::action_at(&start::layout(&self.geo), pos) {
                self.on_start_action(a);
            }
            return EventResult::Handled;
        }
        if self.screen_t() < 0.5 {
            return EventResult::Handled;
        }
        if let Some(ov) = self.overlay() {
            // только что открытые итоги не ловят тот же тап («Играть снова»
            // случайно под пальцем)
            if ov == Overlay::Over && self.now - self.over_card_at < 0.4 {
                return EventResult::Handled;
            }
            if let Some(a) = self.card_action_at(pos) {
                self.apply(a);
            }
            return EventResult::Handled;
        }
        if self.fx.over_at.is_some() && !self.over_card {
            // партия окончена: меню доступно, любое другое касание — к итогам
            if self.geo.menu_btn.contains(pos) {
                self.menu_open = true;
                audio::play(Sfx::Click);
            } else if self.over_waiting() {
                self.show_over_card();
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
        if self.screen == Screen::Start {
            return match key {
                Key::Enter | Key::Space | Key::Num1 => {
                    self.on_start_action(StartAction::Play(Variant::Standard));
                    EventResult::Handled
                }
                Key::Num2 => {
                    self.on_start_action(StartAction::Play(Variant::Deluxe));
                    EventResult::Handled
                }
                _ => EventResult::Ignored,
            };
        }
        match key {
            Key::Escape => {
                if self.over_card && !self.menu_open {
                    self.over_card = false;
                } else if self.drag.is_some() {
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
                } else if self.over_waiting() {
                    self.show_over_card();
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

    /// Атлас загрузился — включить текстурную отрисовку отделок.
    fn poll_atlas(&mut self) {
        if self.atlas.is_some() {
            return;
        }
        let (Some(store), Some(h)) = (&self.image_store, self.atlas_handle) else { return };
        let state = store.lock().ok().and_then(|st| st.state_of(h));
        match state {
            Some(ImageLoadState::Ready) => self.atlas = Some(TextureId(h.0)),
            Some(ImageLoadState::Failed) => {
                log::warn!("атлас отделок не загрузился — отделки рисуются цветом");
                self.atlas_handle = None;
            }
            _ => {}
        }
    }

    fn tick(&mut self, dt: f32) {
        self.now += dt;
        // safe area может прийти (или смениться) без изменения размера окна —
        // тогда layout элемента не перезапускается, а отступы нужны новые
        if self.geo.safe != syngui::viewport::safe_area() {
            self.geo = layout::compute(self.bounds);
        }
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
        let hover_slot = if self.drag.is_none() && !self.input_locked() && self.screen == Screen::Game { self.geo.slot_at(self.pointer) } else { None };
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

        if self.over_waiting() && !self.over_sound_played {
            self.over_sound_played = true;
            audio::play(Sfx::Over);
            self.say(Voice::GameOver, now);
        }
        let want_overlay = if self.overlay().is_some() { 1.0 } else { 0.0 };
        self.fx.overlay_t = anim::damp(self.fx.overlay_t, want_overlay, 11.0, dt);
        if (self.fx.overlay_t - want_overlay).abs() < 0.004 {
            self.fx.overlay_t = want_overlay;
        }

        // частицы исчезающих кубиков
        let mut bursts: Vec<(Rect, world::Look)> = Vec::new();
        for i in 0..self.fx.vanish.len() {
            let v = self.fx.vanish[i];
            if !v.spawned && now >= v.born + v.delay {
                self.fx.vanish[i].spawned = true;
                bursts.push((self.geo.cube_rect(v.r, v.c), self.cell_look(v.color, v.r, v.c)));
            }
        }
        for (rect, look) in bursts {
            self.burst(rect, look);
        }
        let due: Vec<Firework> = self.fx.fireworks.iter().copied().filter(|f| f.at <= now).collect();
        if !due.is_empty() {
            self.fx.fireworks.retain(|f| f.at > now);
            for f in due {
                self.launch_firework(f);
            }
        }
        if self.skin.map(|s| s.finished(now)).unwrap_or(false) {
            self.skin = None;
        }
        if self.skin_prev.map(|s| s.finished(now)).unwrap_or(false) {
            self.skin_prev = None;
        }
        if game.scoring.combo >= 3 && !game.over && !self.menu_open {
            self.embers(dt);
        }
        self.fx.particles.update(dt);

        // режим «по кругу»: микшер сам перешёл к следующему треку
        // Сверяемся только при смене трека у микшера: команда set_track
        // доходит до аудиопотока не сразу, и сравнение «в лоб» откатывало бы
        // только что выбранный кнопкой трек.
        let cur = audio::current_track();
        let mixer_switched = cur != self.seen_track;
        self.seen_track = cur;
        if mixer_switched && self.stats().loop_all && cur as u8 != self.stats().track {
            let mut st = self.stats();
            st.track = cur as u8;
            self.ctx.stats.set(st);
            self.stats_dirty = true;
        }
        if self.game_dirty && !self.frozen {
            self.game_dirty = false;
            persist::save_game(self.variant, &self.game());
        }
        if self.stats_dirty && now - self.last_save > 1.5 {
            persist::save(&self.stats());
            self.stats_dirty = false;
            self.last_save = now;
        }
    }

    /// Конфетти: цветные квадратики сыплются сверху поля («Люкс» — золотые лепестки).
    fn confetti(&mut self, n: usize) {
        let g = self.geo;
        let elv = self.variant.is_deluxe();
        for i in 0..n {
            let color = if elv { theme::hex(theme::GOLD).lerp(&theme::hex("#ffe9a8"), self.prng.unit()) } else { theme::cell_color(1 + (i % 7) as u8) };
            let x = g.board.origin.x + self.prng.range(-0.05, 1.05) * g.board.width();
            let y = g.board.origin.y - self.prng.range(0.0, 60.0);
            self.fx.particles.push(Particle {
                pos: Point::new(x, y),
                vel: (self.prng.range(-60.0, 60.0), self.prng.range(40.0, 160.0)),
                size: self.prng.range(5.0, 10.0),
                color: color.lighten(self.prng.range(0.0, 0.3)).with_alpha(0.95),
                life: self.prng.range(1.6, 2.6),
                gravity: if elv { 90.0 } else { 140.0 },
                drag: if elv { 1.8 } else { 1.2 },
                round: i % 3 == 0,
                shape: if elv { PShape::Petal } else { PShape::Square },
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

    /// Искры вдоль очищенной линии.
    fn line_sparkles(&mut self, is_row: bool, idx: usize, base: Color) {
        for k in 0..SIZE {
            let (r, c) = if is_row { (idx, k) } else { (k, idx) };
            let cr = self.geo.cell_rect(r, c);
            let (vx, vy) = if is_row { (self.prng.range(-30.0, 30.0), self.prng.range(-120.0, -40.0)) } else { (self.prng.range(-120.0, 120.0), self.prng.range(-40.0, 10.0)) };
            self.fx.particles.push(Particle {
                pos: Point::new(cr.origin.x + cr.width() * self.prng.range(0.2, 0.8), cr.origin.y + cr.height() * self.prng.range(0.2, 0.8)),
                vel: (vx, vy),
                size: self.prng.range(3.0, 6.0),
                color: if k % 2 == 0 { Color::WHITE } else { base.lighten(0.5) }.with_alpha(0.95),
                life: self.prng.range(0.4, 0.8),
                gravity: 60.0,
                drag: 2.5,
                shape: PShape::Spark,
                ..Default::default()
            });
        }
    }

    /// Частицы очистки по виду отделки («Люкс»).
    fn burst_finish(&mut self, cx: f32, cy: f32, size: f32, base: Color, accent: Color, kind: Burst) {
        use std::f32::consts::TAU;
        let n = match kind {
            Burst::Coins => 8,
            Burst::Shards => 9,
            Burst::Chips => 7,
            Burst::Dust => 10,
            Burst::Embers => 10,
            Burst::Sparks => 12,
            Burst::Fibres => 9,
            Burst::Sprinkles => 10,
        };
        for i in 0..n {
            let u = self.prng.unit();
            let (vel, size_px, color, life, gravity, drag, shape, round) = match kind {
                Burst::Coins => {
                    let ang = self.prng.range(-2.6, -0.5);
                    let sp = self.prng.range(120.0, 300.0);
                    ((ang.cos() * sp, ang.sin() * sp), self.prng.range(4.0, 7.0), base.lighten(0.25).lerp(&accent, u * 0.6), self.prng.range(0.7, 1.2), 560.0, 0.8, PShape::Coin, false)
                }
                Burst::Shards => {
                    let ang = self.prng.range(0.0, TAU);
                    let sp = self.prng.range(80.0, 280.0);
                    ((ang.cos() * sp, ang.sin() * sp - 60.0), self.prng.range(3.0, 7.0), if i % 3 == 0 { Color::WHITE } else { accent.lerp(&base, 0.5) }, self.prng.range(0.5, 1.0), 380.0, 1.4, if i % 3 == 0 { PShape::Spark } else { PShape::Square }, false)
                }
                Burst::Chips => {
                    let ang = self.prng.range(0.0, TAU);
                    let sp = self.prng.range(60.0, 220.0);
                    ((ang.cos() * sp, ang.sin() * sp - 80.0), self.prng.range(3.0, 6.0), base.lerp(&accent, u * 0.6), self.prng.range(0.5, 0.9), 520.0, 1.2, if i % 2 == 0 { PShape::BarH } else { PShape::BarV }, false)
                }
                Burst::Dust => {
                    let ang = self.prng.range(0.0, TAU);
                    let sp = self.prng.range(30.0, 140.0);
                    ((ang.cos() * sp, ang.sin() * sp - 30.0), self.prng.range(2.0, 5.0), base.lerp(&accent, u * 0.8).with_alpha(0.85), self.prng.range(0.4, 0.8), 200.0, 2.5, PShape::Round, true)
                }
                Burst::Embers => {
                    let ang = self.prng.range(0.0, TAU);
                    let sp = self.prng.range(40.0, 200.0);
                    ((ang.cos() * sp, ang.sin() * sp - 90.0), self.prng.range(3.0, 8.0), accent.lerp(&Color::WHITE, u * 0.4), self.prng.range(0.5, 1.1), -60.0, 1.8, if i % 3 == 0 { PShape::Spark } else { PShape::Round }, true)
                }
                Burst::Sparks => {
                    let ang = self.prng.range(0.0, TAU);
                    let sp = self.prng.range(120.0, 320.0);
                    ((ang.cos() * sp, ang.sin() * sp), self.prng.range(2.0, 5.0), if i % 2 == 0 { Color::WHITE } else { base.lighten(0.4) }, self.prng.range(0.3, 0.7), 80.0, 2.0, PShape::Spark, false)
                }
                Burst::Fibres => {
                    let ang = self.prng.range(0.0, TAU);
                    let sp = self.prng.range(30.0, 120.0);
                    ((ang.cos() * sp, ang.sin() * sp - 40.0), self.prng.range(2.5, 5.0), base.lerp(&accent, u * 0.7), self.prng.range(0.7, 1.3), 90.0, 2.2, PShape::Petal, false)
                }
                Burst::Sprinkles => {
                    let ang = self.prng.range(0.0, TAU);
                    let sp = self.prng.range(70.0, 240.0);
                    ((ang.cos() * sp, ang.sin() * sp - 70.0), self.prng.range(2.5, 5.0), theme::cell_color(1 + (i % 7) as u8).lighten(0.2), self.prng.range(0.5, 1.0), 420.0, 1.4, if i % 2 == 0 { PShape::BarH } else { PShape::Round }, i % 2 == 1)
                }
            };
            let _ = size;
            self.fx.particles.push(Particle {
                pos: Point::new(cx, cy),
                vel,
                size: size_px,
                color: color.with_alpha(color.a.min(0.95)),
                life,
                gravity,
                drag,
                round,
                shape,
                ..Default::default()
            });
        }
    }

    fn burst(&mut self, rect: Rect, look: world::Look) {
        let material = look.material;
        let base = look.base;
        let cx = rect.origin.x + rect.width() * 0.5;
        let cy = rect.origin.y + rect.height() * 0.5;
        if let Some(f) = look.finish {
            self.burst_finish(cx, cy, rect.width(), base, theme::hex(f.accent), f.burst);
            return;
        }
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
                        color: if i % 3 == 0 { Color::WHITE } else { theme::material_accent(Material::Crystal).lerp(&base, 0.55) }.with_alpha(0.95),
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
                        color: base.lighten(0.25).lerp(&Color::WHITE, self.prng.unit() * 0.5).with_alpha(0.98),
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
                        color: match look.accent {
                            Some(a) => a.lerp(&Color::WHITE, hot * 0.4),
                            None => theme::hex("#ff5a1f").lerp(&theme::hex("#ffd166"), hot),
                        }
                        .with_alpha(0.95),
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
            Event::MouseDown { button: MouseButton::Left, .. } if self.touch_pressed => {
                self.touch_pressed = false;
                EventResult::Handled
            }
            Event::MouseDown { button: MouseButton::Left, position } => self.on_press(*position, false, None),
            Event::MouseUp { button: MouseButton::Left, position } => self.on_release(*position),
            Event::TouchStart { id, position } => {
                self.touch_pressed = true;
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
                if self.screen == Screen::Start {
                    EventResult::Ignored
                } else if self.menu_open {
                    self.menu_open = false;
                    EventResult::Handled
                } else if self.over_card {
                    self.over_card = false;
                    EventResult::Handled
                } else {
                    self.go_home();
                    EventResult::Handled
                }
            }
            Event::AppSuspended => {
                audio::suspend();
                if !self.frozen && self.screen == Screen::Game {
                    persist::save_game(self.variant, &self.game());
                    persist::save(&self.stats());
                }
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
            || (self.screen == Screen::Start && start::action_at(&start::layout(&self.geo), self.pointer).is_some())
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
        self.poll_atlas();
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
    fn mount(&mut self, tree: &mut ElementTree) {
        // атлас отделок: декодируется в фоне, готовность опрашивает `poll_atlas`
        if let Some(store) = tree.image_store.clone() {
            if let Ok(mut st) = store.lock() {
                let (h, _) = st.request(&ImageSource::Bytes { key: "elle/atlas".into(), data: Arc::new(ATLAS_BYTES.to_vec()) });
                self.atlas_handle = Some(h);
            }
            self.image_store = Some(store);
        }
    }
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
