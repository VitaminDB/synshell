//! Проверка привязки элементов к config.toml без окна: страница строится в
//! тестовом дереве syngui, по элементам «щёлкают» — файл должен измениться.

use syngui::prelude::*;
use syngui::testing::{click_at, TestHarness};

use crate::{pages, state, store};

fn center(h: &TestHarness, id: ElementId) -> Point {
    let b = h.element_bounds(id);
    Point::new(b.origin.x + b.size.width / 2.0, b.origin.y + b.size.height / 2.0)
}

#[test]
fn clicks_reach_config_file() {
    let dir = std::env::temp_dir().join(format!("synsettings-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    store::init(&path);
    let ctx = state::init("windows");
    provide_context(ctx);

    // «Поведение окон»: первый переключатель — «Поднимать окно при фокусе» (true).
    let mut h = TestHarness::new((pages::find("windows").build)());
    h.frame(None, 1000.0, 3000.0);
    let toggles = h.find_by_type_name("Toggle");
    assert!(!toggles.is_empty(), "на странице нет переключателей");
    let before = store::config().windows.raise_on_focus;
    let p = center(&h, toggles[0]);
    h.send_events(&click_at(p));
    h.frame(None, 1000.0, 3000.0);
    assert_eq!(store::config().windows.raise_on_focus, !before, "клик не дошёл до конфига");

    // Сохранение с задержкой: файл появился и содержит значение, комментарии
    // встроенного образца на месте.
    std::thread::sleep(std::time::Duration::from_millis(700));
    let text = std::fs::read_to_string(&path).expect("config.toml не записан");
    assert!(text.contains(&format!("raise_on_focus = {}", !before)), "{text}");
    assert!(text.contains("# Оболочка: панели"), "комментарии потеряны");

    // «Панели»: кнопка «Добавить панель» добавляет [[panel]].
    let mut h = TestHarness::new((pages::find("panels").build)());
    h.frame(None, 1000.0, 6000.0);
    let n = store::config().panels.len();
    let add = h
        .find_by_type_name("Button")
        .into_iter()
        .find(|id| {
            h.tree
                .get(*id)
                .map(|e| format!("{:?}", e.accessibility_info()).contains("Добавить панель"))
                .unwrap_or(false)
        });
    let add = add.expect("кнопка «Добавить панель» не найдена");
    h.send_events(&click_at(center(&h, add)));
    assert_eq!(store::config().panels.len(), n + 1);

    // «Комбинации клавиш»: захват нажатия через корневой перехватчик.
    ctx.page.set("shortcuts".into());
    let mut h = TestHarness::new(crate::app::root(ctx));
    h.frame(None, 1200.0, 900.0);
    let combo = h
        .find_by_type_name("Button")
        .into_iter()
        .find(|id| {
            h.tree
                .get(*id)
                .map(|e| format!("{:?}", e.accessibility_info()).contains("Записать сочетание"))
                .unwrap_or(false)
        })
        .expect("кнопка захвата не найдена");
    h.send_events(&click_at(center(&h, combo)));
    assert_eq!(ctx.capture.get_untracked().as_deref(), Some("+new"));
    h.send_event(&Event::KeyDown(Key::F7));
    assert_eq!(ctx.capture.get_untracked(), None, "захват не завершился");
    h.frame(None, 1200.0, 900.0);
    let caught = h.find_by_type_name("Button").into_iter().any(|id| {
        h.tree.get(id).map(|e| format!("{:?}", e.accessibility_info()).contains("\"F7\"")).unwrap_or(false)
    });
    assert!(caught, "сочетание F7 не показано на кнопке");

    // «Темы»: клик по карточке Nord (вторая, после стандартной) применяет
    // тему — сбрасывает свой акцент, цвета палитры и обоев, ставит форму темы.
    store::set(&[store::Seg::K("appearance"), store::Seg::K("accent")], "#ff0000");
    store::set(&[store::Seg::K("appearance"), store::Seg::K("colors"), store::Seg::K("bg")], "#000000");
    let mut h = TestHarness::new((pages::find("themes").build)());
    h.frame(None, 1100.0, 4000.0);
    let cards = h.find_by_type_name("GestureDetector");
    assert!(cards.len() >= 15, "карточек тем: {}", cards.len());
    h.send_events(&click_at(center(&h, cards[1])));
    let c = store::config();
    assert_eq!(c.appearance.theme, "nord");
    assert!(c.appearance.resolved.is_some());
    assert_eq!(c.appearance.accent, "");
    assert!(c.appearance.colors.is_empty(), "{:?}", c.appearance.colors);
    assert_eq!(c.appearance.corner_radius, 12.0);
    assert_eq!(c.appearance.palette().accent.hex(), "#88c0d0");
    assert_eq!(c.decorations.active_color, "theme");
    assert_eq!(c.wallpaper.color, "");
    // Карточка «Стандартная» возвращает оформление по умолчанию.
    let mut h = TestHarness::new((pages::find("themes").build)());
    h.frame(None, 1100.0, 4000.0);
    let cards = h.find_by_type_name("GestureDetector");
    h.send_events(&click_at(center(&h, cards[0])));
    let c = store::config();
    assert_eq!(c.appearance.theme, "");
    assert!(c.appearance.resolved.is_none());
    assert_eq!(c.appearance.corner_radius, 10.0);
    let _ = std::fs::remove_dir_all(&dir);
}
