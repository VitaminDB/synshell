//! Телефон: режим окон, домашний экран, «назад»; жесты.

use synshell_common::action::MobileMode;
use synshell_common::ipc::Request;
use synshell_common::Action;
use syngui::prelude::*;

use crate::op;
use crate::store;
use crate::sys;
use crate::ui::*;

const MODES: &[(&str, &str)] = &[("pages", n_!("Страницы")), ("free", n_!("Свободный стол"))];

/// Режим окон: в конфиг и сразу композитору (без перезапуска).
fn mode_row(current: MobileMode) -> W {
    let mut dd = Dropdown::new().width(220.0);
    for (v, l) in MODES {
        dd = dd.item(DropdownItem::new(*v, tl(l)));
    }
    row(
        t!("Режим окон"),
        t!("Страницы — приложения во весь экран, листание вбок; плитки — окна друг под другом; свободный — окна двигаются по большому столу. Меняется и удержанием на рабочем столе"),
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
    let pb = &c.power_button;
    page(
        t!("Телефон"),
        t!("Режимы окон и домашний экран оболочки synmobile-shell."),
        vec![
            group(
                &t!("Окна"),
                vec![
                    mode_row(m.mode),
                    choice_row(
                        t!("Виртуальный стол"),
                        t!("Размер стола в свободном режиме; двигается двумя пальцами"),
                        op!["mobile", "desk"],
                        &m.desk,
                        &[("2x2", n_!("2 × 2 экрана")), ("3x3", n_!("3 × 3 экрана")), ("infinite", n_!("Бесконечный"))],
                    ),
                    choice_row(
                        t!("Ручка окна"),
                        t!("За неё окно двигают и меняют размер в свободном режиме"),
                        op!["mobile", "handle"],
                        &m.handle,
                        &[("server", n_!("Полоса композитора")), ("none", n_!("Нет (удержание по окну)"))],
                    ),
                    int_row(t!("Высота ручки"), t!("Логические пиксели"), op!["mobile", "handle_height"], m.handle_height as i64, 16, 64, 2),
                    switch_row(t!("Масштаб стола щипком"), t!("Свободный режим: щипок уменьшает стол, тап по окну — обратно 1:1"), op!["mobile", "pinch_zoom"], m.pinch_zoom),
                    switch_row(t!("Запоминать режим окон"), t!("Выбранный режим сохраняется в config.toml"), op!["mobile", "remember_mode"], m.remember_mode),
                ],
            ),
            group(
                &t!("Пробуждение"),
                vec![
                    switch_row(t!("Двойной стук"), t!("Два касания по погашенному экрану включают его"), op!["mobile", "double_tap_wake"], m.double_tap_wake),
                    switch_row(t!("Поднять, чтобы разбудить"), t!("Экран включается, когда телефон берут в руки"), op!["mobile", "raise_to_wake"], m.raise_to_wake),
                    switch_row(t!("Экраном вниз — «Не беспокоить»"), t!("Положите телефон экраном вниз — уведомления не беспокоят, пока его не поднимут"), op!["mobile", "flip_to_dnd"], m.flip_to_dnd),
                ],
            ),
            group(
                &t!("Поворот экрана"),
                vec![
                    switch_row(t!("Автоповорот"), t!("Экран поворачивается за телефоном (акселерометр)"), op!["rotation", "auto"], rot.auto),
                    switch_row(
                        t!("Кнопка «повернуть»"),
                        t!("Когда ориентация зафиксирована, а телефон повернули, — кнопка в углу на несколько секунд"),
                        op!["rotation", "suggest"],
                        rot.suggest,
                    ),
                    switch_row(t!("Вверх ногами"), t!("Поворачивать и на 180°"), op!["rotation", "upside_down"], rot.upside_down),
                    int_row(
                        t!("Задержка поворота, мс"),
                        t!("Сколько телефон должен пробыть в новом положении — случайный наклон не поворачивает экран"),
                        op!["rotation", "delay_ms"],
                        rot.delay_ms as i64,
                        0,
                        5000,
                        100,
                    ),
                    int_row(
                        t!("Угол срабатывания, °"),
                        t!("Насколько наклонить телефон, чтобы экран повернулся; меньше — чувствительнее (по умолчанию 35)"),
                        op!["rotation", "threshold_deg"],
                        rot.threshold_deg as i64,
                        10,
                        80,
                        5,
                    ),
                    int_row(
                        t!("Анимация поворота, мс"),
                        t!("0 — поворачивать сразу, без анимации"),
                        op!["rotation", "animation_ms"],
                        rot.animation_ms as i64,
                        0,
                        1500,
                        50,
                    ),
                ],
            ),
            group(
                &t!("Экранная клавиатура"),
                vec![slider_row(
                    t!("Масштаб клавиатуры"),
                    t!("Отдельно от масштаба оболочки: высота клавиш и подписи; меняется сразу"),
                    op!["osk", "scale"],
                    c.osk.scale as f64,
                    0.5,
                    2.0,
                    0.05,
                    2,
                )],
            ),
            group(
                &t!("Кнопка питания"),
                vec![
                    choice_row(
                        t!("Нажатие"),
                        t!("Как в Android: погасить экран и заблокировать, повторное нажатие — включить"),
                        op!["power_button", "short"],
                        &pb.short,
                        &[
                            ("", n_!("По умолчанию (погасить и заблокировать)")),
                            ("screen-toggle", n_!("Погасить и заблокировать")),
                            ("shell power-menu", n_!("Меню выключения")),
                            ("none", n_!("Ничего")),
                        ],
                    ),
                    choice_row(
                        t!("Удержание"),
                        t!("Срабатывает, пока кнопку держат"),
                        op!["power_button", "long"],
                        &pb.long,
                        &[
                            ("", n_!("По умолчанию (меню выключения)")),
                            ("shell power-menu", n_!("Меню выключения")),
                            ("screen-toggle", n_!("Погасить и заблокировать")),
                            ("none", n_!("Ничего")),
                        ],
                    ),
                    int_row(t!("Удержание, мс"), t!("0 — 3000 мс"), op!["power_button", "long_ms"], pb.long_ms as i64, 0, 10000, 250),
                ],
            ),
            group(
                &t!("Назад"),
                vec![choice_row(
                    t!("Клавиша «назад» для окна"),
                    t!("Её получает приложение по жесту «назад», если поверх нет окна оболочки"),
                    op!["mobile", "back_key"],
                    &m.back_key,
                    &[("XF86Back", n_!("XF86Back (браузеры, GTK4, Qt)")), ("Escape", "Escape"), ("Alt+Left", "Alt+←"), ("close", n_!("Закрыть окно"))],
                )],
            ),
            group(
                &t!("Домашний экран"),
                vec![
                    if c.widgets.is_some() {
                        widget_reset_row()
                    } else {
                        switch_row(t!("Страница ресурсов"), t!("Первая страница: процессор, память, питание, запущенные приложения"), op!["mobile", "resources_page"], m.resources_page)
                    },
                    int_row(&t!("Колонок в сетке"), "", op!["mobile", "home_columns"], m.home_columns as i64, 2, 8, 1),
                    choice_row(
                        t!("Вид «Пуска»"),
                        t!("Переключается и кнопкой внизу «Пуска»"),
                        op!["mobile", "launcher"],
                        &m.launcher,
                        &[("pages", n_!("Значки по страницам")), ("list", n_!("Список"))],
                    ),
                ],
            ),
            note(&t!("Приложения на домашнем экране: удержание по значку в «Пуске» → «На домашний экран»; пустой список — все приложения по алфавиту. Сетка видна, если включены «Значки на рабочем столе» (Оформление → Обои).")),
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
        t!("Вибрация"),
        t!("Виброотклик телефона: жесты, экранная клавиатура, касания в приложениях, уведомления."),
        vec![
            group(
                "",
                vec![
                    switch_row(t!("Вибрация"), t!("Общий выключатель виброотклика"), op!["haptics", "enabled"], h.enabled),
                    row(t!("Сила"), t!("Насколько сильно вибрирует отклик"), slider),
                    row_inline(
                        t!("Проверить"),
                        t!("Короткая вибрация с выбранной силой"),
                        button(&t!("Проверить"), || {
                            let s = store::config().haptics.strength;
                            synshell_common::haptics::test(s);
                        }),
                    ),
                ],
            ),
            group(
                &t!("Отклик на"),
                vec![
                    switch_row(t!("Жесты"), t!("«Назад», «домой», шторка; «Недавние» удержанием — сильнее"), op!["haptics", "gestures"], h.gestures),
                    switch_row(t!("Клавиатура"), t!("Нажатия клавиш экранной клавиатуры"), op!["haptics", "keyboard"], h.keyboard),
                    switch_row(t!("Касания"), t!("Удержание пальцем (выбор, меню), переключатели"), op!["haptics", "touch"], h.touch),
                    switch_row(t!("Уведомления"), t!("Кроме режима «Не беспокоить»"), op!["haptics", "notifications"], h.notifications),
                ],
            ),
        ],
    )
}

/// Действия для жестов (значение — строка действия как в `[keybindings]`).
const GESTURE_ACTIONS: &[(&str, &str)] = &[
    ("back", n_!("Назад")),
    ("shell home", n_!("Домой (свернуть окна и открыть приложения)")),
    ("minimize-all", n_!("Свернуть все окна")),
    ("shell recents", n_!("Недавние")),
    ("shell shade", n_!("Шторка")),
    ("shell launcher", n_!("«Пуск»")),
    ("page next", n_!("Следующее приложение")),
    ("page prev", n_!("Предыдущее приложение")),
    ("mobile-mode-cycle", n_!("Следующий режим окон")),
    ("overview", n_!("Обзор окон")),
    ("close", n_!("Закрыть окно")),
    ("key Escape", n_!("Клавиша Escape")),
    ("shell keyboard", n_!("Экранная клавиатура")),
    ("none", n_!("Ничего")),
];

fn action_row(label: impl AsRef<str>, hint: impl AsRef<str>, key: &str, current: &Action) -> W {
    let cur = current.to_string();
    let mut opts: Vec<(String, String)> = GESTURE_ACTIONS.iter().map(|(v, l)| (v.to_string(), tl(l))).collect();
    if !opts.iter().any(|(v, _)| *v == cur) {
        opts.push((cur.clone(), t!("Своё: {cur}", cur = cur)));
    }
    row(label, hint, choice_owned(op!["gestures", key.to_string()], &cur, opts, 240.0))
}

pub fn gestures() -> W {
    let c = store::config();
    let g = &c.gestures;
    page(
        t!("Жесты"),
        t!("Жесты сенсорного экрана и тачпада. Действия — те же, что у сочетаний клавиш."),
        vec![
            group(
                "",
                vec![
                    switch_row(t!("Жесты включены"), t!("Выключено — касания уходят приложениям как есть"), op!["gestures", "enabled"], g.enabled),
                    int_row(t!("Зона у края"), t!("Ширина полосы у края экрана, в которой начинается жест, px"), op!["gestures", "edge_size"], g.edge_size as i64, 8, 64, 2),
                    int_row(t!("Порог жеста"), t!("Путь пальца до распознавания, px"), op!["gestures", "threshold"], g.threshold as i64, 16, 160, 4),
                    int_row(t!("Долгое нажатие"), t!("Удержание = правая кнопка, мс"), op!["gestures", "long_press_ms"], g.long_press_ms as i64, 200, 1500, 50),
                ],
            ),
            group(
                &t!("От краёв экрана"),
                vec![
                    action_row(&t!("Свайп от левого края"), "", "edge_left", &g.edge_left),
                    action_row(&t!("Свайп от правого края"), "", "edge_right", &g.edge_right),
                    action_row(&t!("Свайп снизу вверх"), "", "edge_bottom", &g.edge_bottom),
                    action_row(t!("Свайп снизу с задержкой"), t!("Провести вверх и задержать палец"), "edge_bottom_hold", &g.edge_bottom_hold),
                    action_row(&t!("Свайп сверху вниз"), "", "edge_top", &g.edge_top),
                ],
            ),
            group(
                &t!("Пальцы"),
                vec![
                    switch_row(t!("Двумя пальцами — стол"), t!("В свободном режиме двигать виртуальный стол"), op!["gestures", "two_finger_pan"], g.two_finger_pan),
                    choice_row(
                        &t!("Удержание по окну"),
                        "",
                        op!["gestures", "long_press_window"],
                        &g.long_press_window,
                        &[("none", n_!("Передать приложению")), ("move", n_!("Перетащить окно"))],
                    ),
                ],
            ),
            group(
                &t!("Тачпад"),
                vec![
                    action_row(&t!("Три пальца вбок"), "", "touchpad_horizontal", &g.touchpad_horizontal),
                    action_row(&t!("Три пальца вверх"), "", "touchpad_up", &g.touchpad_up),
                ],
            ),
        ],
    )
}
