//! Телефон: режим окон, домашний экран, «назад»; жесты.

use synshell_common::action::MobileMode;
use synshell_common::ipc::Request;
use synshell_common::Action;
use syngui::prelude::*;

use crate::op;
use crate::store;
use crate::sys;
use crate::ui::*;

const MODES: &[(&str, &str)] = &[("pages", "Страницы"), ("free", "Свободный стол")];

/// Режим окон: в конфиг и сразу композитору (без перезапуска).
fn mode_row(current: MobileMode) -> W {
    let mut dd = Dropdown::new().width(220.0);
    for (v, l) in MODES {
        dd = dd.item(DropdownItem::new(*v, *l));
    }
    row(
        "Режим окон",
        "Страницы — приложения во весь экран, листание вбок; плитки — окна друг под другом; свободный — окна двигаются по большому столу. Меняется и удержанием на рабочем столе",
        dd.selected(current.as_str().to_string()).on_change(|v: &str| {
            set(&op!["mobile", "mode"], v.to_string());
            if let Ok(m) = v.parse::<MobileMode>() {
                std::thread::spawn(move || {
                    sys::ipc(Request::Action { action: Action::MobileMode(m) });
                });
            }
        }),
    )
}

pub fn phone() -> W {
    let c = store::config();
    let m = &c.mobile;
    let rot = &c.rotation;
    page(
        "Телефон",
        "Режимы окон и домашний экран оболочки synmobile-shell.",
        vec![
            group(
                "Окна",
                vec![
                    mode_row(m.mode),
                    choice_row(
                        "Виртуальный стол",
                        "Размер стола в свободном режиме; двигается двумя пальцами",
                        op!["mobile", "desk"],
                        &m.desk,
                        &[("2x2", "2 × 2 экрана"), ("3x3", "3 × 3 экрана"), ("infinite", "Бесконечный")],
                    ),
                    choice_row(
                        "Ручка окна",
                        "За неё окно двигают и меняют размер в свободном режиме",
                        op!["mobile", "handle"],
                        &m.handle,
                        &[("server", "Полоса композитора"), ("none", "Нет (удержание по окну)")],
                    ),
                    int_row("Высота ручки", "Логические пиксели", op!["mobile", "handle_height"], m.handle_height as i64, 16, 64, 2),
                    switch_row("Масштаб стола щипком", "Дорого на CPU-композиторе", op!["mobile", "pinch_zoom"], m.pinch_zoom),
                    switch_row("Запоминать режим окон", "Выбранный режим сохраняется в config.toml", op!["mobile", "remember_mode"], m.remember_mode),
                ],
            ),
            group(
                "Поворот экрана",
                vec![
                    switch_row("Автоповорот", "Экран поворачивается за телефоном (акселерометр)", op!["rotation", "auto"], rot.auto),
                    switch_row(
                        "Кнопка «повернуть»",
                        "Когда ориентация зафиксирована, а телефон повернули, — кнопка в углу на несколько секунд",
                        op!["rotation", "suggest"],
                        rot.suggest,
                    ),
                    switch_row("Вверх ногами", "Поворачивать и на 180°", op!["rotation", "upside_down"], rot.upside_down),
                ],
            ),
            group(
                "Назад",
                vec![choice_row(
                    "Клавиша «назад» для окна",
                    "Её получает приложение по жесту «назад», если поверх нет окна оболочки",
                    op!["mobile", "back_key"],
                    &m.back_key,
                    &[("XF86Back", "XF86Back (браузеры, GTK4, Qt)"), ("Escape", "Escape"), ("Alt+Left", "Alt+←"), ("close", "Закрыть окно")],
                )],
            ),
            group(
                "Домашний экран",
                vec![
                    switch_row("Страница ресурсов", "Первая страница: процессор, память, питание, запущенные приложения", op!["mobile", "resources_page"], m.resources_page),
                    int_row("Колонок в сетке", "", op!["mobile", "home_columns"], m.home_columns as i64, 2, 8, 1),
                    choice_row(
                        "Вид «Пуска»",
                        "Переключается и кнопкой внизу «Пуска»",
                        op!["mobile", "launcher"],
                        &m.launcher,
                        &[("pages", "Значки по страницам"), ("list", "Список")],
                    ),
                ],
            ),
            note("Приложения на домашнем экране: удержание по значку в «Пуске» → «На домашний экран»; пустой список — все приложения по алфавиту. Сетка видна, если включены «Значки на рабочем столе» (Оформление → Обои)."),
        ],
    )
}

/// Виброотклик (`[haptics]`).
pub fn vibration() -> W {
    let c = store::config();
    let h = c.haptics.clone();
    let strength = h.strength;
    let p = op!["haptics", "strength"];
    let slider = Slider::new()
        .range(10.0, 100.0)
        .step(5.0)
        .value(strength as f32)
        .show_value(0)
        .width(200.0)
        .on_change(move |v| set(&p, v.round() as i64));
    page(
        "Вибрация",
        "Виброотклик телефона: жесты, экранная клавиатура, касания в приложениях, уведомления.",
        vec![
            group(
                "",
                vec![
                    switch_row("Вибрация", "Общий выключатель виброотклика", op!["haptics", "enabled"], h.enabled),
                    row("Сила", "Насколько сильно вибрирует отклик", slider),
                    row_inline(
                        "Проверить",
                        "Короткая вибрация с выбранной силой",
                        button("Проверить", || {
                            let s = store::config().haptics.strength;
                            synshell_common::haptics::test(s);
                        }),
                    ),
                ],
            ),
            group(
                "Отклик на",
                vec![
                    switch_row("Жесты", "«Назад», «домой», шторка; «Недавние» удержанием — сильнее", op!["haptics", "gestures"], h.gestures),
                    switch_row("Клавиатура", "Нажатия клавиш экранной клавиатуры", op!["haptics", "keyboard"], h.keyboard),
                    switch_row("Касания", "Удержание пальцем (выбор, меню), переключатели", op!["haptics", "touch"], h.touch),
                    switch_row("Уведомления", "Кроме режима «Не беспокоить»", op!["haptics", "notifications"], h.notifications),
                ],
            ),
        ],
    )
}

/// Действия для жестов (значение — строка действия как в `[keybindings]`).
const GESTURE_ACTIONS: &[(&str, &str)] = &[
    ("back", "Назад"),
    ("shell home", "Домой (свернуть окна и открыть приложения)"),
    ("minimize-all", "Свернуть все окна"),
    ("shell recents", "Недавние"),
    ("shell shade", "Шторка"),
    ("shell launcher", "«Пуск»"),
    ("page next", "Следующее приложение"),
    ("page prev", "Предыдущее приложение"),
    ("mobile-mode-cycle", "Следующий режим окон"),
    ("overview", "Обзор окон"),
    ("close", "Закрыть окно"),
    ("key Escape", "Клавиша Escape"),
    ("shell keyboard", "Экранная клавиатура"),
    ("none", "Ничего"),
];

fn action_row(label: &str, hint: &str, key: &str, current: &Action) -> W {
    let cur = current.to_string();
    let mut opts: Vec<(String, String)> = GESTURE_ACTIONS.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
    if !opts.iter().any(|(v, _)| *v == cur) {
        opts.push((cur.clone(), format!("Своё: {cur}")));
    }
    row(label, hint, choice_owned(op!["gestures", key.to_string()], &cur, opts, 240.0))
}

pub fn gestures() -> W {
    let c = store::config();
    let g = &c.gestures;
    page(
        "Жесты",
        "Жесты сенсорного экрана и тачпада. Действия — те же, что у сочетаний клавиш.",
        vec![
            group(
                "",
                vec![
                    switch_row("Жесты включены", "Выключено — касания уходят приложениям как есть", op!["gestures", "enabled"], g.enabled),
                    int_row("Зона у края", "Ширина полосы у края экрана, в которой начинается жест, px", op!["gestures", "edge_size"], g.edge_size as i64, 8, 64, 2),
                    int_row("Порог жеста", "Путь пальца до распознавания, px", op!["gestures", "threshold"], g.threshold as i64, 16, 160, 4),
                    int_row("Долгое нажатие", "Удержание = правая кнопка, мс", op!["gestures", "long_press_ms"], g.long_press_ms as i64, 200, 1500, 50),
                ],
            ),
            group(
                "От краёв экрана",
                vec![
                    action_row("Свайп от левого края", "", "edge_left", &g.edge_left),
                    action_row("Свайп от правого края", "", "edge_right", &g.edge_right),
                    action_row("Свайп снизу вверх", "", "edge_bottom", &g.edge_bottom),
                    action_row("Свайп снизу с задержкой", "Провести вверх и задержать палец", "edge_bottom_hold", &g.edge_bottom_hold),
                    action_row("Свайп сверху вниз", "", "edge_top", &g.edge_top),
                ],
            ),
            group(
                "Пальцы",
                vec![
                    switch_row("Двумя пальцами — стол", "В свободном режиме двигать виртуальный стол", op!["gestures", "two_finger_pan"], g.two_finger_pan),
                    choice_row(
                        "Удержание по окну",
                        "",
                        op!["gestures", "long_press_window"],
                        &g.long_press_window,
                        &[("none", "Передать приложению"), ("move", "Перетащить окно")],
                    ),
                ],
            ),
            group(
                "Тачпад",
                vec![
                    action_row("Три пальца вбок", "", "touchpad_horizontal", &g.touchpad_horizontal),
                    action_row("Три пальца вверх", "", "touchpad_up", &g.touchpad_up),
                ],
            ),
        ],
    )
}
