//! Стартовая страница: логотип, две карточки видов игры («Стандарт» и
//! «Эльвира») и переключатели звука. Геометрия одна для отрисовки и
//! хит-теста (как `overlay.rs`).

use super::cube::{draw_cube, CubeStyle};
use super::finish;
use super::layout::{inset, offset, scaled_about_center, Geo};
use super::paint::{icon, text, text_nowrap, text_shadowed};
use super::theme::{self, hex};
use super::world;
use super::View;
use crate::anim::{self, clamp01};
use crate::game::Material;
use crate::persist::Variant;
use syngui::mss::TextAlign;
use syngui::prelude::*;
use syngui::render::display_list::Border;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartAction {
    Play(Variant),
    ToggleSound,
    ToggleMusic,
    ToggleVoice,
}

#[derive(Clone, Copy, Debug)]
pub struct StartGeo {
    pub title: Rect,
    pub subtitle: Rect,
    /// Карточки: [Стандарт, Эльвира].
    pub cards: [Rect; 2],
    /// Переключатели: звуки, музыка, голос.
    pub toggles: [Rect; 3],
    pub version: Rect,
}

pub const CARD_STD_H: f32 = 166.0;
pub const CARD_ELV_H: f32 = 196.0;

pub fn layout(geo: &Geo) -> StartGeo {
    let col = geo.col;
    let w = col.width().min(420.0);
    let x = col.origin.x + (col.width() - w) * 0.5;
    let title_h = 66.0;
    let sub_h = 22.0;
    let toggles_h = 40.0;
    let total = title_h + sub_h + 26.0 + CARD_STD_H + 14.0 + CARD_ELV_H + 26.0 + toggles_h + 8.0 + 16.0;
    let content = geo.content;
    let mut y = content.origin.y + ((content.height() - total) * 0.42).max(12.0);
    let title = Rect::new(Point::new(x, y), Size::new(w, title_h));
    y += title_h;
    let subtitle = Rect::new(Point::new(x, y), Size::new(w, sub_h));
    y += sub_h + 26.0;
    let card_std = Rect::new(Point::new(x, y), Size::new(w, CARD_STD_H));
    y += CARD_STD_H + 14.0;
    let card_elv = Rect::new(Point::new(x, y), Size::new(w, CARD_ELV_H));
    y += CARD_ELV_H + 26.0;
    let gap = 8.0;
    let tw = (w - gap * 2.0) / 3.0;
    let toggles = [
        Rect::new(Point::new(x, y), Size::new(tw, toggles_h)),
        Rect::new(Point::new(x + tw + gap, y), Size::new(tw, toggles_h)),
        Rect::new(Point::new(x + (tw + gap) * 2.0, y), Size::new(tw, toggles_h)),
    ];
    y += toggles_h + 8.0;
    let version = Rect::new(Point::new(x, y), Size::new(w, 16.0));
    StartGeo { title, subtitle, cards: [card_std, card_elv], toggles, version }
}

pub fn action_at(sg: &StartGeo, p: Point) -> Option<StartAction> {
    if sg.cards[0].contains(p) {
        return Some(StartAction::Play(Variant::Standard));
    }
    if sg.cards[1].contains(p) {
        return Some(StartAction::Play(Variant::Elvira));
    }
    if sg.toggles[0].contains(p) {
        return Some(StartAction::ToggleSound);
    }
    if sg.toggles[1].contains(p) {
        return Some(StartAction::ToggleMusic);
    }
    if sg.toggles[2].contains(p) {
        return Some(StartAction::ToggleVoice);
    }
    None
}

/// Отрисовка стартовой страницы с общей прозрачностью `alpha` и сдвигом `dy`.
pub fn draw(v: &View, list: &mut DisplayList, alpha: f32, dy: f32) {
    if alpha <= 0.005 {
        return;
    }
    let sg = layout(&v.geo);
    let b = v.geo.bounds;
    let pal = world::Palette::of(&world::WORLDS[world::VELVET]);
    let t = v.now;
    if alpha < 0.999 {
        list.push_opacity(alpha);
    }
    // фон «Бархат» с боке
    list.push_gradient_rect(
        b,
        Gradient::Linear { angle_deg: 180.0, stops: vec![ColorStop::new(pal.top, 0.0), ColorStop::new(pal.bottom, 1.0)] },
        [0.0; 4],
    );
    world::draw_ambient(list, world::Ambient::Bokeh, b, t, 1.0, pal.accent);
    // золотые искры
    world::draw_ambient(list, world::Ambient::Fireflies, b, t * 0.6, 0.35, pal.accent);

    // логотип: золото «дышит» к белому, под ним сияние
    let title = offset(sg.title, 0.0, dy);
    let breathe = (t * 0.9).sin() * 0.5 + 0.5;
    let gold = hex(theme::GOLD);
    let glow_r = inset(Rect::new(Point::new(title.origin.x + title.width() * 0.15, title.origin.y + 8.0), Size::new(title.width() * 0.7, title.height() - 16.0)), 0.0);
    list.push_glow_shadow(glow_r, gold.with_alpha(0.10 + 0.08 * breathe), 34.0, (0.0, 0.0), [30.0; 4]);
    text_shadowed(list, "ELLE BLOCK", title, gold.lerp(&Color::WHITE, 0.15 + 0.25 * breathe), 46.0, 800, TextAlign::CENTER);
    let sub = offset(sg.subtitle, 0.0, dy);
    text_nowrap(list, "головоломка 8×8  ·  выберите вид игры", sub, hex(theme::TEXT_DIM).with_alpha(0.9), 13.0, 600, TextAlign::CENTER);

    let stats = v.stats();
    for (i, variant) in Variant::ALL.iter().enumerate() {
        let rect = offset(sg.cards[i], 0.0, dy);
        let hot = sg.cards[i].contains(v.pointer);
        let press = v.start_press.filter(|(k, _)| *k == i).map(|(_, born)| 1.0 - clamp01((t - born) / 0.35)).unwrap_or(0.0);
        let scale = 1.0 + 0.015 * if hot { 1.0 } else { 0.0 } + 0.03 * anim::punch(1.0 - press);
        let rect = scaled_about_center(rect, scale);
        let radius = [22.0; 4];
        let elv = variant.is_elvira();
        // карточка
        let (c0, c1, edge) = if elv {
            (hex("#4a1f52"), hex("#2a1030"), hex("#d9b25a"))
        } else {
            (hex("#3d4fc8"), hex("#232a5e"), hex(theme::CARD_EDGE))
        };
        list.push_shadow(rect, Color::from_srgb(0, 0, 0, 0.45), 24.0, (0.0, 12.0), radius);
        if elv {
            let pulse = (t * 1.3).sin() * 0.5 + 0.5;
            list.push_glow_shadow(rect, edge.with_alpha(0.10 + 0.10 * pulse + 0.3 * press), 26.0, (0.0, 0.0), radius);
        }
        list.push_gradient_rect(
            rect,
            Gradient::Linear { angle_deg: 160.0, stops: vec![ColorStop::new(c0.lighten(if hot { 0.08 } else { 0.0 }), 0.0), ColorStop::new(c1, 1.0)] },
            radius,
        );
        list.push_outline(inset(rect, 1.0), edge.with_alpha(if hot { 0.95 } else { 0.7 }), if elv { 1.8 } else { 1.2 }, 0.0, radius);
        if press > 0.0 {
            list.push_rect(rect, Color::WHITE.with_alpha(0.25 * press), radius);
        }
        // превью слева
        let pv = Rect::new(Point::new(rect.origin.x + 16.0, rect.origin.y + 16.0), Size::new(124.0, rect.height() - 32.0));
        preview(v, list, pv, *variant, i);
        // текст справа
        let tx = pv.origin.x + pv.width() + 16.0;
        let tw = rect.origin.x + rect.width() - 16.0 - tx;
        let name = Rect::new(Point::new(tx, rect.origin.y + 14.0), Size::new(tw, 34.0));
        text_shadowed(list, &variant.name().to_uppercase(), name, if elv { hex("#ffe9a8") } else { Color::WHITE }, 26.0, 800, TextAlign::LEFT | TextAlign::VCENTER);
        // рекорд под названием
        let best = stats.best_variant(*variant);
        let rec_y = rect.origin.y + 48.0;
        let trophy = Rect::new(Point::new(tx, rec_y), Size::new(20.0, 20.0));
        icon(list, theme::ICON_TROPHY, trophy, hex(theme::GOLD), 15.0);
        let best_r = Rect::new(Point::new(tx + 24.0, rec_y), Size::new(tw - 24.0, 20.0));
        text_nowrap(list, &format!("Рекорд  {best}"), best_r, hex(theme::GOLD), 13.5, 800, TextAlign::LEFT | TextAlign::VCENTER);
        // описание
        let desc = Rect::new(Point::new(tx, rect.origin.y + 72.0), Size::new(tw, rect.height() - 72.0 - 44.0));
        let desc_text = if elv { "24 материала — золото, стекло, велюр, дуб… — блеск, анимации и английские реплики." } else { "Классика: чистые цвета, режимы блоков, честная раздача." };
        text(list, desc_text, desc, hex(theme::TEXT_DIM).with_alpha(0.92), 12.5, 500, TextAlign::LEFT);
        // кнопка-подсказка в правом нижнем углу
        let label = if v.start_saved[i] { "Продолжить" } else { "Играть" };
        let bw = 118.0;
        let btn = Rect::new(Point::new(rect.origin.x + rect.width() - 16.0 - bw, rect.origin.y + rect.height() - 14.0 - 30.0), Size::new(bw, 30.0));
        let bcol = if elv { hex("#d9b25a") } else { hex(theme::BTN) };
        list.push_rect(btn, bcol.with_alpha(if hot { 1.0 } else { 0.85 }), [15.0; 4]);
        let ir = Rect::new(Point::new(btn.origin.x + 8.0, btn.origin.y), Size::new(20.0, btn.height()));
        let fg = if elv { hex("#2a1030") } else { Color::WHITE };
        icon(list, theme::ICON_PLAY, ir, fg, 16.0);
        let lr = Rect::new(Point::new(btn.origin.x + 28.0, btn.origin.y), Size::new(bw - 34.0, btn.height()));
        text_nowrap(list, label, lr, fg, 13.0, 800, TextAlign::CENTER);
    }

    // переключатели
    let labels = ["Звуки", "Музыка", "Голос"];
    let on = [stats.sound, stats.music, stats.voice];
    for i in 0..3 {
        let rect = offset(sg.toggles[i], 0.0, dy);
        let hot = sg.toggles[i].contains(v.pointer);
        let col = hex(if hot { theme::BTN_SECONDARY_HOVER } else { theme::BTN_SECONDARY });
        list.push_rect_bordered(rect, col.with_alpha(0.9), [20.0; 4], Border { width: 1.0, color: Color::WHITE.with_alpha(0.18) });
        let ir = Rect::new(Point::new(rect.origin.x + 10.0, rect.origin.y), Size::new(22.0, rect.height()));
        icon(list, if on[i] { theme::ICON_CHECK_ON } else { theme::ICON_CHECK_OFF }, ir, Color::WHITE.with_alpha(if on[i] { 1.0 } else { 0.5 }), 18.0);
        let lr = Rect::new(Point::new(rect.origin.x + 34.0, rect.origin.y), Size::new(rect.width() - 40.0, rect.height()));
        text_nowrap(list, labels[i], lr, Color::WHITE.with_alpha(if on[i] { 1.0 } else { 0.6 }), 13.5, 800, TextAlign::LEFT | TextAlign::VCENTER);
    }
    let ver = offset(sg.version, 0.0, dy);
    text_nowrap(list, &format!("Elle Block {}  ·  syngui", env!("CARGO_PKG_VERSION")), ver, hex(theme::TEXT_DIM).with_alpha(0.55), 10.5, 600, TextAlign::CENTER);
    if alpha < 0.999 {
        list.pop_opacity();
    }
}

/// Превью вида игры: 2×2 кубика. «Стандарт» — цвета с покачиванием, «Эльвира» —
/// материалы, которые по очереди переворачиваются в следующий.
fn preview(v: &View, list: &mut DisplayList, area: Rect, variant: Variant, card: usize) {
    let t = v.now;
    let s = (area.width().min(area.height()) * 0.44).max(18.0);
    let gap = s * 0.14;
    let x0 = area.origin.x + (area.width() - s * 2.0 - gap) * 0.5;
    let y0 = area.origin.y + (area.height() - s * 2.0 - gap) * 0.5;
    for k in 0..4u32 {
        let (r, c) = (k / 2, k % 2);
        let mut rect = Rect::new(Point::new(x0 + c as f32 * (s + gap), y0 + r as f32 * (s + gap)), Size::new(s, s));
        let seed = 900 + card as u32 * 10 + k;
        match variant {
            Variant::Standard => {
                let bob = ((t * 1.6 + k as f32 * 1.4).sin()) * 1.5;
                rect = offset(rect, 0.0, bob);
                let color = theme::cell_color([1u8, 3, 2, 5][k as usize]);
                let style = CubeStyle::default().with_material(Material::Plain, seed, t);
                draw_cube(list, rect, color, &CubeStyle { flash: 0.15 * super::fx::Fx::sheen(t, r as usize, c as usize), ..style });
            }
            Variant::Elvira => {
                // переворот: каждые 2.6 с один из кубиков меняет материал
                let period = 2.6;
                let phase = (t / period + k as f32 * 0.25) % 1.0;
                let step = ((t / period + k as f32 * 0.25).floor() as i64).max(0) as u32;
                let flip = clamp01((phase - 0.82) / 0.18);
                let sx = (flip * std::f32::consts::PI).cos().abs().max(0.06);
                let shown = if flip < 0.5 { step } else { step + 1 };
                let id = 1 + ((shown * 7 + k * 5 + card as u32) % crate::game::FINISH_COUNT as u32) as u8;
                let f = finish::get(id);
                let base = finish::base_color(f, theme::cell_color(1 + ((k + shown) % 7) as u8));
                let w = s * sx;
                let rr = Rect::new(Point::new(rect.origin.x + (s - w) * 0.5, rect.origin.y), Size::new(w, s));
                let look = world::Look { material: Material::Plain, base, accent: Some(hex(f.accent)), finish: Some(f), bonus: false };
                let style = CubeStyle::default().with_look(&look, seed + shown, t).with_texture(v.atlas);
                draw_cube(list, rr, base, &CubeStyle { flash: 0.25 * (1.0 - sx), ..style });
            }
        }
    }
}
