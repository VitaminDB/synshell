//! Игровой режим (gameshell, `[gameshell]`): переключение режима оболочки на
//! лету, окна во весь экран, библиотека игр, вид лаунчера, геймпад.

use synshell_common::action::FormFactorTarget;
use synshell_common::ipc::{Request, Response};
use synshell_common::Action;
use syngui::prelude::*;

use crate::op;
use crate::store;
use crate::sys;
use crate::ui::*;

const MODES: &[(&str, &str)] = &[("desktop", n_!("Рабочий стол")), ("phone", n_!("Телефон")), ("game", n_!("Игровой (gameshell)"))];

/// Режим сейчас — у композитора; меняется сразу, без перезапуска сеанса.
fn current_row() -> W {
    let current = match sys::ipc(Request::FormFactor) {
        Some(Response::FormFactor { current, .. }) => current,
        _ => store::config().process_form_factor().as_str().to_string(),
    };
    let mut dd = Dropdown::new().width(240.0);
    for (v, l) in MODES {
        dd = dd.item(DropdownItem::new(*v, tl(l)));
    }
    row(
        t!("Режим оболочки сейчас"),
        t!("Меняется на лету: окна подстраиваются, оболочка перезапускается. Из игрового режима — «Выйти из игрового режима» в его меню (Guide или Super)"),
        dd.selected(current).on_change(|v: &str| {
            if let Ok(t) = v.parse::<FormFactorTarget>() {
                std::thread::spawn(move || {
                    sys::ipc(Request::Action { action: Action::FormFactor(t) });
                });
            }
        }),
    )
}

pub fn game() -> W {
    let c = store::config();
    let gs = &c.gameshell;
    page(
        t!("Игровой режим"),
        t!("Полноэкранный лаунчер игр (syngame-shell): все приложения во весь экран без рамок, как на игровой консоли; управление геймпадом, клавиатурой и касаниями."),
        vec![
            group(
                &t!("Режим"),
                vec![
                    current_row(),
                    choice_row(
                        t!("При запуске сеанса"),
                        t!("[platform] form_factor; правка переключает и текущий сеанс"),
                        op!["platform", "form_factor"],
                        &c.platform.form_factor,
                        &[("auto", n_!("Как у устройства")), ("desktop", n_!("Рабочий стол")), ("phone", n_!("Телефон")), ("game", n_!("Игровой"))],
                    ),
                    choice_row(
                        t!("Выход из игрового режима"),
                        t!("Куда ведёт «Выйти из игрового режима»"),
                        op!["gameshell", "exit_to"],
                        &gs.exit_to,
                        &[("device", n_!("Режим устройства")), ("previous", n_!("Откуда пришли")), ("desktop", n_!("Рабочий стол")), ("phone", n_!("Телефон"))],
                    ),
                    switch_row(t!("Запоминать режим"), t!("Переключение действием сохраняется в config.toml — следующий сеанс начнётся в нём"), op!["gameshell", "remember_mode"], gs.remember_mode),
                ],
            ),
            group(
                &t!("Окна"),
                vec![
                    switch_row(t!("Приложения во весь экран"), t!("Главные окна — во весь экран без рамок и заголовков, диалоги — по центру поверх"), op!["gameshell", "force_fullscreen"], gs.force_fullscreen),
                    list_row(t!("Исключения"), t!("app_id окон, которым полный экран не навязывается (можно *)"), op!["gameshell", "fullscreen_exclude"], &gs.fullscreen_exclude, "org.example.Tool, *calc*"),
                    switch_row(t!("Спрашивать перед закрытием игры"), t!("«Закрыть игру» в меню — с подтверждением"), op!["gameshell", "confirm_close"], gs.confirm_close),
                ],
            ),
            group(
                &t!("Библиотека"),
                vec![
                    switch_row(t!("Игры из меню приложений"), t!("Программы с категориями ниже"), op!["gameshell", "desktop_games"], gs.desktop_games),
                    list_row(t!("Категории игр"), t!("Категории .desktop, которые считаются играми"), op!["gameshell", "game_categories"], &gs.game_categories, "Game"),
                    list_row(t!("Тоже игры"), t!("Id .desktop без категории Game"), op!["gameshell", "include_apps"], &gs.include_apps, "org.example.App"),
                    switch_row(t!("Библиотека Steam"), t!("Установленные игры и обложки из кэша Steam"), op!["gameshell", "steam"], gs.steam),
                    list_row(t!("Каталоги Steam"), t!("Где лежит steamapps; пусто — стандартные"), op!["gameshell", "steam_dirs"], &gs.steam_dirs, "~/.local/share/Steam"),
                    text_row(t!("Запуск игры Steam"), t!("{appid} — номер игры"), op!["gameshell", "steam_launch"], &gs.steam_launch, "steam steam://rungameid/{appid}"),
                    switch_row(t!("Прятать служебное Steam"), t!("Proton, Steam Linux Runtime, Redistributables"), op!["gameshell", "steam_hide_tools"], gs.steam_hide_tools),
                    list_row(t!("Скрытые"), t!("Id игр: steam:228980, org.foo.Game, custom:имя; прячутся и кнопкой X → «Скрыть»"), op!["gameshell", "hide"], &gs.hide, ""),
                    int_row(t!("Недавних на главной"), "", op!["gameshell", "recent_count"], gs.recent_count as i64, 1, 40, 1),
                ],
            ),
            group(
                &t!("Вид"),
                vec![
                    list_row(
                        t!("Вкладки"),
                        t!("По порядку: home, games, favorites, steam, apps, tag:<метка>"),
                        op!["gameshell", "tabs"],
                        &gs.tabs,
                        "home, games, steam, apps",
                    ),
                    text_row(t!("Вкладка при запуске"), t!("Ключ из списка вкладок"), op!["gameshell", "start_tab"], &gs.start_tab, "home"),
                    choice_row(t!("Размер плиток"), "", op!["gameshell", "tile_size"], &gs.tile_size, &[("small", n_!("Маленькие")), ("medium", n_!("Средние")), ("large", n_!("Большие"))]),
                    choice_row(t!("Плитки"), "", op!["gameshell", "tile_style"], &gs.tile_style, &[("cover", n_!("Обложки")), ("icon", n_!("Значки"))]),
                    switch_row(t!("Афиша на главной"), t!("Большая картинка и кнопка «Играть» выбранной игры"), op!["gameshell", "hero"], gs.hero),
                    choice_row(
                        t!("Фон"),
                        "",
                        op!["gameshell", "background"],
                        &gs.background,
                        &[("hero", n_!("Афиша выбранной игры")), ("wallpaper", n_!("Обои")), ("color", n_!("Цвет"))],
                    ),
                    row(t!("Цвет фона"), "", color_field(op!["gameshell", "background_color"], &gs.background_color, "#0b0e14")),
                    list_row(t!("Строка состояния"), t!("network, battery, volume, gamepad, clock"), op!["gameshell", "status"], &gs.status, ""),
                    switch_row(t!("Подсказки кнопок"), t!("Внизу экрана — для геймпада и клавиатуры"), op!["gameshell", "hints"], gs.hints),
                    switch_row(t!("Панели и доки"), t!("Обычные панели [[panel]] и в игровом режиме"), op!["gameshell", "panels"], gs.panels),
                ],
            ),
            group(
                &t!("Звуки"),
                vec![
                    switch_row(t!("Звуки интерфейса"), t!("Навигация, выбор, назад, вкладки, меню, запуск игры"), op!["gameshell", "sounds"], gs.sounds),
                    row_wide(
                        t!("Звуковая тема"),
                        t!("Своя для игрового режима или как у оболочки"),
                        super::audio::theme_choice(op!["gameshell", "sound_theme"], &gs.sound_theme, Some(t!("Как в «Звук → Звуки событий»").to_string())),
                    ),
                    int_row(t!("Громкость звуков, %"), "", op!["gameshell", "sound_volume"], gs.sound_volume as i64, 0, 150, 5),
                    row(
                        t!("Прослушать"),
                        t!("Навигация, выбор, вкладка, меню, запуск"),
                        button(t!("Прослушать"), || {
                            let c = store::config();
                            let th = if c.gameshell.sound_theme.trim().is_empty() { c.sound.theme.clone() } else { c.gameshell.sound_theme.clone() };
                            super::audio::preview(
                                th,
                                &["gameshell-navigate", "gameshell-navigate", "gameshell-select", "gameshell-tab", "gameshell-menu-open", "gameshell-back", "gameshell-launch"],
                                c.gameshell.sound_volume as f32 / 100.0,
                            )
                        }),
                    ),
                ],
            ),
            group(
                &t!("Управление"),
                vec![
                    switch_row(t!("Геймпад"), t!("Навигация крестовиной и стиком, A — запуск, B — назад, LB/RB — вкладки"), op!["gameshell", "gamepad"], gs.gamepad),
                    choice_row(
                        t!("Кнопка Guide и Super в игре"),
                        t!("Select+Start — то же, что Guide"),
                        op!["gameshell", "guide_button"],
                        &gs.guide_button,
                        &[("overlay", n_!("Быстрое меню")), ("home", n_!("Сразу на главную")), ("none", n_!("Ничего"))],
                    ),
                ],
            ),
            note(&t!("Свои игры — в config.toml: [[gameshell.game]] name = \"Pony Town\", command = \"firefox --kiosk https://pony.town\", cover = \"~/Картинки/pony.png\", tags = [\"web\"]. С id найденной игры (steam:570, org.foo.Game) запись меняет её имя, обложку, афишу и команду.")),
        ],
    )
}
