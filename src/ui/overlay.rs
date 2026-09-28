//! Геометрия карточек-оверлеев (меню и конец игры). Используется и для
//! отрисовки, и для хит-теста — одна функция, никаких расхождений.

use super::layout::Geo;
use syngui::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Resume,
    NewGame,
    ToggleSound,
    ToggleMusic,
    Difficulty(usize),
    TrackPrev,
    TrackNext,
    ToggleLoopAll,
    /// Скрыть итоги и посмотреть поле, на котором кончились ходы.
    ViewBoard,
}

#[derive(Clone, Copy, Debug)]
pub struct CardButton {
    pub rect: Rect,
    pub action: Action,
    pub primary: bool,
    pub label: &'static str,
    pub icon: &'static str,
}

#[derive(Clone, Debug)]
pub struct Card {
    pub rect: Rect,
    pub title: Rect,
    pub body: Rect,
    pub buttons: Vec<CardButton>,
}

pub const BTN_H: f32 = 50.0;

fn card_rect(geo: &Geo, height: f32) -> Rect {
    let w = (geo.col.width() - 8.0).min(340.0);
    let x = geo.board.origin.x + (geo.board.width() - w) * 0.5;
    // центр карточки — центр поля, но не выше HUD и не ниже окна
    let cy = geo.board.origin.y + geo.board.height() * 0.5;
    let max_y = geo.content.origin.y + geo.content.height() - height - 8.0;
    let y = (cy - height * 0.5).min(max_y).max(geo.top.origin.y + 4.0);
    Rect::new(Point::new(x, y), Size::new(w, height))
}

/// (действие, основная, подпись, иконка, доля ширины в ряду)
type Spec = (Action, bool, &'static str, &'static str, f32);

fn layout(geo: &Geo, height: f32, title_h: f32, body_h: f32, rows: &[&[Spec]]) -> Card {
    let rect = card_rect(geo, height);
    let pad = 22.0;
    let title = Rect::new(Point::new(rect.origin.x + pad, rect.origin.y + pad), Size::new(rect.width() - pad * 2.0, title_h));
    let body = Rect::new(
        Point::new(rect.origin.x + pad, title.origin.y + title_h + 10.0),
        Size::new(rect.width() - pad * 2.0, body_h),
    );
    let mut buttons = Vec::new();
    let mut y = body.origin.y + body_h + 16.0;
    let full = rect.width() - pad * 2.0;
    for row in rows {
        let n = row.len().max(1) as f32;
        let gap = 10.0;
        let total_weight: f32 = row.iter().map(|r| r.4).sum::<f32>().max(0.001);
        let avail = full - gap * (n - 1.0);
        let mut x = rect.origin.x + pad;
        for (action, primary, label, icon, weight) in row.iter() {
            let w = avail * weight / total_weight;
            buttons.push(CardButton { rect: Rect::new(Point::new(x, y), Size::new(w, BTN_H)), action: *action, primary: *primary, label, icon });
            x += w + gap;
        }
        y += BTN_H + 10.0;
    }
    Card { rect, title, body, buttons }
}

/// Карточка меню/паузы.
pub fn menu_card(geo: &Geo) -> Card {
    use super::theme::{ICON_CHECK_OFF, ICON_LOOP, ICON_NEXT, ICON_PLAY, ICON_PREV, ICON_REPLAY};
    let title_h = 40.0;
    let body_h = 3.0 * 26.0 + 26.0;
    let height = 20.0 + title_h + 8.0 + body_h + 12.0 + (BTN_H + 10.0) * 5.0 + 8.0;
    layout(
        geo,
        height,
        title_h,
        body_h,
        &[
            &[(Action::Resume, true, "Продолжить", ICON_PLAY, 1.0)],
            &[(Action::NewGame, false, "Новая игра", ICON_REPLAY, 1.0)],
            &[
                (Action::Difficulty(0), false, "Легко", "", 1.0),
                (Action::Difficulty(1), false, "Норма", "", 1.0),
                (Action::Difficulty(2), false, "Сложно", "", 1.0),
            ],
            &[
                (Action::TrackPrev, false, "", ICON_PREV, 0.55),
                (Action::TrackNext, false, "Трек", "", 2.2),
                (Action::TrackNext, false, "", ICON_NEXT, 0.55),
                (Action::ToggleLoopAll, false, "", ICON_LOOP, 0.55),
            ],
            &[(Action::ToggleSound, false, "Звуки", ICON_CHECK_OFF, 1.0), (Action::ToggleMusic, false, "Музыка", ICON_CHECK_OFF, 1.0)],
        ],
    )
}

/// Карточка конца игры.
pub fn over_card(geo: &Geo) -> Card {
    use super::theme::{ICON_REPLAY, ICON_VIEW};
    let title_h = 44.0;
    let body_h = 150.0;
    let height = 22.0 + title_h + 10.0 + body_h + 16.0 + BTN_H + 10.0 + 12.0;
    layout(geo, height, title_h, body_h, &[&[(Action::ViewBoard, false, "Поле", ICON_VIEW, 0.8), (Action::NewGame, true, "Играть снова", ICON_REPLAY, 1.2)]])
}
