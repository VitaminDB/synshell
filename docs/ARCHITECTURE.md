# synshell — архитектура

Полноценное окружение рабочего стола для Wayland на Rust. Интерфейс оболочки
и настроек — на фреймворке [syngui](../../syngui). Устроено как Plasma:
композитор (как KWin) + отдельный процесс оболочки (как plasmashell) +
приложение настроек (как systemsettings). Падение оболочки не роняет сеанс:
композитор перезапускает её.

```
┌─────────────────────────── synwm (композитор, smithay 0.7) ───────────────────────────┐
│ backends: udev/DRM+libinput+libseat (сеанс) · winit (вложенное окно для разработки)       │
│ протоколы: xdg-shell, xdg-decoration, wlr-layer-shell, xdg-output, dmabuf, viewporter,     │
│  fractional-scale, presentation, data-device, primary-selection, wlr-data-control,         │
│  pointer-constraints, relative-pointer, cursor-shape, idle-notify, idle-inhibit,           │
│  ext-session-lock, xdg-activation, foreign-toplevel-list, keyboard-shortcuts-inhibit,      │
│  text-input/input-method, virtual-keyboard, single-pixel-buffer, security-context          │
│ оконный менеджер: столы, плавающие/плиточные раскладки, snap, правила окон, анимации,      │
│  серверные рамки (свой растеризатор), тени, Alt+Tab, обзор                                 │
│ IPC: $XDG_RUNTIME_DIR/synwm.$WAYLAND_DISPLAY.sock (JSON-строки)                        │
└──────────────▲───────────────────────────────▲─────────────────────────────────────────────┘
               │ wayland (layer-shell)         │ IPC (EventStream, Action, WindowAction)
┌──────────────┴───────────────────────────────┴─────────────────┐   ┌─────────────────────────┐
│ syndesktop-shell (syngui через syngui-layer)                   │   │ synsettings     │
│  обои · панели и доки · меню запуска · уведомления (D-Bus)      │   │ (syngui + winit)        │
│  OSD громкости/яркости · меню питания · блокировка · Alt+Tab     │   │ правит config.toml      │
└─────────────────────────────────────────────────────────────────┘   └─────────────────────────┘
                 все читают ~/.config/synshell/config.toml (+ тема, theme.mss) и следят
```

## Крейты

| Крейт | Что это |
|---|---|
| `synshell-common` | `Config` (весь `config.toml`), `Action` (строковые действия), `KeyCombo`, IPC-протокол и клиент, пути XDG, опрос изменений файлов, `.desktop` и темы значков (`xdg`), типы файлов и программы для них (`mime`: shared-mime-info, `mimeapps.list`) |
| `synwm` | композитор + CLI `synwm msg …` |
| `syngui-layer` | хост syngui на layer-shell поверхностях (sctk 0.19 + wgpu-surface из `wl_surface`) — «winit для оболочки» |
| `syndesktop-shell` | оболочка рабочего стола |
| `synmobile-shell` | оболочка телефона: строка состояния, навигация, домашний экран — [MOBILE.md](MOBILE.md) |
| `synkeyboard` | экранная клавиатура (layer-shell + `zwp_virtual_keyboard_v1`, автопоказ по `zwp_input_method_v2`) — [MOBILE.md](MOBILE.md) |
| `synsettings` | «Параметры системы» |
| `synfiles` | проводник (syngui + winit): вкладки, две панели, поиск, корзина, операции в фоне; `--viewer` — просмотрщик картинок (модуль `viewer`, сцена из synthos) — [FILES.md](FILES.md) |
| `synshot` | снимок с выбором (syngui-layer, слой overlay на каждом выводе): кадры всех выводов, окна и указатель берутся у композитора одним IPC `capture` (по Print композитор снимает сразу и передаёт `--capture ФАЙЛ`), оверлей — свой элемент (затемнение, рамка, ручки, лупа), результат — PNG / wl-copy, уведомление через `shell screenshot-*` |

## Конфигурация

Один файл `~/.config/synshell/config.toml`, схема — `synshell-common/src/config.rs`,
документированный пример — `synshell-common/default-config.toml` (пишется при первом
запуске). Все поля необязательны. Композитор и оболочка опрашивают mtime раз в секунду
(`watch::FileWatcher`) и применяют изменения на лету. Настройки меняют файл через
`toml_edit`, сохраняя комментарии пользователя.

Оформление: тема `appearance.theme` (`synshell-common/src/theme.rs`, встроенные — в
`synshell-common/themes/<id>/`, свои — `~/.config/synshell/themes/<id>/`) даёт палитру
вариантов `[dark]`/`[light]`, обои, цвета заголовков, свои переменные и MSS. Тема загружается
в `Config::parse` (`Appearance::resolved`), поэтому `Appearance::palette()` у всех процессов
уже с её цветами; поверх — `accent` и `[appearance.colors]`. Оболочка собирает MSS слоями:
`Appearance::mss_variables()` (`--bg --surface --surface-alt --panel-bg --menu-bg --fg
--muted --border --accent --accent-fg --accent-soft --hover --pressed --danger --success
--warning --radius --radius-sm --font-size`) → `--shadow`/`--scrim` и `vars` темы →
встроенный `shell.mss` → `shell.mss` темы → фон рабочего стола → `~/.config/synshell/theme.mss`.
Композитор берёт `palette()`, `titlebar_colors()` и `wallpaper_color()` для рамок и фона.
Композитор и оболочка следят и за файлами активной темы. Подробно — [THEMES.md](THEMES.md).

## Док и режим редактирования

`[[panel]] mode = "dock"` — поверхность `syndesktop-dock` на всю длину края и с запасом
над полосой (увеличение значков, подпись); область ввода (`syngui_layer::set_input_region`)
— только полоса, клики над ней проходят к окнам (композитор ищет поверхность под курсором
по всем layer-поверхностям слоя с учётом их input region). Ряд значков — `syngui::widgets::Fisheye`,
частицы — `ParticleEmitter`, 3D — MSS `rotate-x/rotate-y`, `background-rotate-x`, `box-reflect`.
Правки из режима редактирования оболочка пишет в `config.toml` сама
(`synshell_common::config_edit`) и сразу перечитывает. Подробно — [DOCK.md](DOCK.md).

## Адаптивная панель и глобальное меню

`[[panel]] defloat = "maximized" | "touch"`: оболочка раз в 150 мс смотрит на окна вывода и
пересоздаёт спецификацию поверхности — без отступа и во всю длину края, класс `.panel-defloated`
вместо `.panel-floating` (без рамки и скруглений) и толщиной `defloated_size` (0 — как `size`);
зона резервирования — толщина панели в текущем состоянии. Обе толщины рекомендует тема (`[panel]`
в `theme.toml`).

Цвета программ (`[appearance] app_colors`): `synshell-common/src/app_theme.rs` строит из палитры
`colors.css`/`gtk.css` GTK, группы `kdeglobals` и `gimp.css`; оболочка (`app_colors.rs`) пишет их в
фоне при загрузке конфига и оповещает Qt/KDE по D-Bus.

Панель-заголовок: апплеты `window-title`, `window-buttons`, `appmenu`
(`syndesktop-shell/src/applets/window.rs`) показывают активное окно своего вывода;
`[windows] borderless_maximized` убирает у развёрнутых окон серверный заголовок
(`Managed::has_titlebar`). Перетаскивание заголовка на панели — `WindowOp::StartMove`:
композитор перехватывает указатель, пока кнопка ещё нажата.

Глобальное меню: композитор реализует `org_kde_kwin_appmenu` (`synwm/src/appmenu.rs`) и
отдаёт адрес меню в `WindowInfo::appmenu`; окна X11 регистрируют меню у
`com.canonical.AppMenu.Registrar` (`syndesktop-shell/src/appmenu.rs`, по `WindowInfo::x11_id`).
Реестр на шине, только пока в конфиге есть апплет `appmenu`: по нему Qt решает, убирать ли
строку меню из окна. Меню читается по `com.canonical.dbusmenu` (AboutToShow + GetLayout,
Event `opened`/`clicked`/`closed`). Qt-программам нужна тема `QT_QPA_PLATFORMTHEME=kde`
(plasma-integration).

GTK3: композитор реализует `gtk_shell1` (`synwm/src/gtk_shell.rs`, протокол —
`protocols/gtk-shell.xml`) и, пока есть апплет `appmenu`, объявляет возможность
`global_menu_bar` — GTK убирает строку меню из окна и сообщает `set_dbus_properties`
(`WindowInfo::gtk_menu`). Оболочка читает модель `org.gtk.Menus` (Start по группам, разделы
через черту) и действия `org.gtk.Actions` (DescribeAll — доступность, флажки, радио;
Activate), `syndesktop-shell/src/gtkmenu.rs`. GIMP 3 отдаёт меню так только с
`GIMP_GTK_MENUBAR=1` — композитор выставляет её детям, пока есть апплет `appmenu`.

## IPC

`synshell-common/src/ipc.rs`. Запрос-ответ одной JSON-строкой; `EventStream` переводит
соединение в поток событий: `Snapshot` → `WindowChanged`/`WindowClosed`/`WindowFocused`/
`WorkspacesChanged`/`OutputsChanged`/`KeyboardLayoutChanged`/`ShellCommand`/
`ConfigReloaded`/`Exiting`.

`Action::Shell(cmd)` композитор не исполняет, а пересылает оболочке событием
`ShellCommand { command }`. Команды оболочки: `launcher`, `run`, `power-menu`,
`notifications`, `clipboard`, `volume ±N`, `mute`, `mic-mute`, `brightness ±N`,
`media play-pause|next|previous`, `lock`, `window-switcher next|prev|commit`,
`edit-panel [N]` / `edit-dock` (режим редактирования панели/дока), `panel-add N`
(окно «Добавить»).

Окружение детей композитора: `WAYLAND_DISPLAY`, `SYNSHELL_SOCKET`,
`XDG_CURRENT_DESKTOP=synshell`, `XDG_SESSION_TYPE=wayland` и `[general.environment]`.

## Оболочка и layer-shell

Пространства имён layer-поверхностей (`namespace`), по ним композитор решает, как с ними
обращаться (исключение из снимков):
`syndesktop-wallpaper` (background), `syndesktop-panel`, `syndesktop-dock` (top), `syndesktop-launcher`,
`syndesktop-popup`, `syndesktop-notification`, `syndesktop-osd` (overlay),
`syndesktop-lock` (через ext-session-lock).

### Анимации оболочки («перетекания»)

Анимации рисует сама оболочка внутри своих поверхностей (композитор
layer-поверхности не анимирует), на примитивах syngui `Presence`,
`AnimatedSwitcher`, `AnimatedPosition`, `AnimatedSize` и перетекании темы
(`docs/07-animation.md` в syngui). `[animations] shell` и `theme_change`
включают их, `enabled`/`speed` задают темп (`crates/syndesktop-shell/src/anim.rs`).

- Всплывающие окна (`popup.rs`): карточка — `Presence::signal(open, …)`; у прижатой
  панели (`PopupAnchor::attached`: не плавающая или `defloat`) якорь растянут до её
  края, зазор нулевой, карточка получает класс `popup-flow-<edge>` и раскрывается
  от панели (`collapse`), а MSS `flow-edge` рисует вогнутые углы цветом панели —
  карточка перетекает в панель. Закрытие «в никуда» играет уход, поверхность
  закрывается по его концу (`on_exit_complete`); смена окна на другое — сразу.
- Меню запуска (`launcher.rs`): список разделов — `AnimatedSwitcher` по номеру раздела
  (снимок выдачи + `version`, чтобы уходящий список не подхватывал новую), строка
  «Выполнить» — `AnimatedSize` с пружиной (`.popup-morph`), полноэкранное меню —
  `Presence`.
- Уведомления (`notifications.rs`): `SHOWN` — карточки на экране, в том числе уходящие;
  `Keyed` по id, `Presence` (въезд справа, уход) внутри `AnimatedPosition` (соседи
  съезжаются на пружине); поверхность закрывается, когда уходящих не осталось.
- OSD (`osd.rs`): `Presence::signal`, поверхность живёт до конца ухода.
- Смена оформления (`main.rs reload_config`): если изменились только `[appearance]`,
  `[wallpaper]`, `[animations]`, поверхности не пересоздаются — таблица стилей
  заменяется с перетеканием цветов (`set_stylesheet_with_transition`), обои
  (`manager.rs`) читают путь из живого конфига и растворяются через
  `AnimatedSwitcher` (`exit_fade(false)`), фон панели пересчитывается из палитры.
  Прочие правки конфига по-прежнему пересобирают оболочку (`generation`).

## Сборка и запуск

```sh
cargo build --release
# вложенно в текущий сеанс (окно winit):
target/release/synwm --nested
# настоящий сеанс: выбрать «synshell» в менеджере входа (data/synshell.desktop)
```
