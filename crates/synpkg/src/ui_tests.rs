//! Меню пакета на карточке обновления без окна: карточка строится в тестовом
//! дереве syngui, правый щелчок и удержание пальцем должны открыть меню.

use synshell_common::Config;
use synsystem::packages::{Source, Update};
use syngui::prelude::*;
use syngui::testing::TestHarness;

use crate::{new_state, ui, St};

fn card() -> (St, TestHarness, Point) {
    let st = new_state(Config::default(), String::new(), use_signal(syngui::window::WindowState::default()));
    let u = Update { name: "zzz-test".into(), old: "1-1".into(), new: "2-1".into(), source: Source::Repo("extra".into()) };
    let mut h = TestHarness::new(ui::update_card(st, u, "описание".into()));
    h.frame(None, 400.0, 300.0);
    let b = h.element_bounds(h.find_by_class("app-card")[0]);
    // Описание: мимо флажка в углу.
    (st, h, Point::new(b.origin.x + 40.0, b.origin.y + b.size.height - 60.0))
}

fn menu_ids(st: St) -> Vec<String> {
    st.menu_items.get_untracked().iter().map(|i| i.id.clone()).collect()
}

#[test]
fn right_click_opens_package_menu() {
    let (st, mut h, p) = card();
    h.send_events(&[Event::MouseMove(p), Event::MouseDown { button: MouseButton::Right, position: p }, Event::MouseUp { button: MouseButton::Right, position: p }]);
    assert!(st.menu_open.get_untracked(), "меню не открылось");
    let ids = menu_ids(st);
    for id in ["details", "q-up", "q-rm", "remove-now", "ignore"] {
        assert!(ids.iter().any(|i| i == id), "нет пункта {id}: {ids:?}");
    }
}

#[test]
fn long_press_opens_package_menu() {
    let (st, mut h, p) = card();
    h.touch_down(1, p);
    std::thread::sleep(std::time::Duration::from_millis(900));
    h.touch_poll();
    h.touch_up(1);
    assert!(st.menu_open.get_untracked(), "удержание не открыло меню");
}
