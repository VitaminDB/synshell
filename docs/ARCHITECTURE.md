# syndesktop — архитектура

Полноценное окружение рабочего стола для Wayland на Rust. Интерфейс оболочки
и настроек — на фреймворке [syngui](../../syngui). Устроено как Plasma:
композитор (как KWin) + отдельный процесс оболочки (как plasmashell) +
приложение настроек (как systemsettings). Падение оболочки не роняет сеанс:
композитор перезапускает её.

```
┌─────────────────────────── syndesktop (композитор, smithay 0.7) ───────────────────────────┐
│ backends: udev/DRM+libinput+libseat (сеанс) · winit (вложенное окно для разработки)       │
│ протоколы: xdg-shell, xdg-decoration, wlr-layer-shell, xdg-output, dmabuf, viewporter,     │
│  fractional-scale, presentation, data-device, primary-selection, wlr-data-control,         │
│  pointer-constraints, relative-pointer, cursor-shape, idle-notify, idle-inhibit,           │
│  ext-session-lock, xdg-activation, foreign-toplevel-list, keyboard-shortcuts-inhibit,      │
│  text-input/input-method, virtual-keyboard, single-pixel-buffer, security-context          │
│ оконный менеджер: столы, плавающие/плиточные раскладки, snap, правила окон, анимации,      │
│  серверные рамки (свой растеризатор), тени, Alt+Tab, обзор                                 │
│ IPC: $XDG_RUNTIME_DIR/syndesktop.$WAYLAND_DISPLAY.sock (JSON-строки)                        │
└──────────────▲───────────────────────────────▲─────────────────────────────────────────────┘
               │ wayland (layer-shell)         │ IPC (EventStream, Action, WindowAction)
┌──────────────┴───────────────────────────────┴─────────────────┐   ┌─────────────────────────┐
│ syndesktop-shell (syngui через syngui-layer)                   │   │ syndesktop-settings     │
│  обои · панели с апплетами · меню запуска · уведомления (D-Bus) │   │ (syngui + winit)        │
│  OSD громкости/яркости · меню питания · блокировка · Alt+Tab     │   │ правит config.toml      │
└─────────────────────────────────────────────────────────────────┘   └─────────────────────────┘
                 все читают ~/.config/syndesktop/config.toml (+ theme.mss) и следят за ним
```

## Крейты

| Крейт | Что это |
|---|---|
| `syndesktop-common` | `Config` (весь `config.toml`), `Action` (строковые действия), `KeyCombo`, IPC-протокол и клиент, пути XDG, опрос изменений файлов |
| `syndesktop` | композитор + CLI `syndesktop msg …` |
| `syngui-layer` | хост syngui на layer-shell поверхностях (sctk 0.19 + wgpu-surface из `wl_surface`) — «winit для оболочки» |
| `syndesktop-shell` | оболочка |
| `syndesktop-settings` | «Параметры системы» |

## Конфигурация

Один файл `~/.config/syndesktop/config.toml`, схема — `syndesktop-common/src/config.rs`,
документированный пример — `syndesktop-common/default-config.toml` (пишется при первом
запуске). Все поля необязательны. Композитор и оболочка опрашивают mtime раз в секунду
(`watch::FileWatcher`) и применяют изменения на лету. Настройки меняют файл через
`toml_edit`, сохраняя комментарии пользователя.

Тема оболочки: встроенный MSS + переменные палитры из `[appearance]`
(`Appearance::mss_variables()`: `--bg --surface --surface-alt --panel-bg --menu-bg --fg
--muted --border --accent --accent-fg --accent-soft --hover --pressed --danger --success
--warning --radius --radius-sm --font-size`) + пользовательский `~/.config/syndesktop/theme.mss`
поверх. Композитор берёт те же цвета из `Appearance::palette()` для рамок и заголовков.

## IPC

`syndesktop-common/src/ipc.rs`. Запрос-ответ одной JSON-строкой; `EventStream` переводит
соединение в поток событий: `Snapshot` → `WindowChanged`/`WindowClosed`/`WindowFocused`/
`WorkspacesChanged`/`OutputsChanged`/`KeyboardLayoutChanged`/`ShellCommand`/
`ConfigReloaded`/`Exiting`.

`Action::Shell(cmd)` композитор не исполняет, а пересылает оболочке событием
`ShellCommand { command }`. Команды оболочки: `launcher`, `run`, `power-menu`,
`notifications`, `clipboard`, `volume ±N`, `mute`, `mic-mute`, `brightness ±N`,
`media play-pause|next|previous`, `lock`, `window-switcher next|prev|commit`.

Окружение детей композитора: `WAYLAND_DISPLAY`, `SYNDESKTOP_SOCKET`,
`XDG_CURRENT_DESKTOP=syndesktop`, `XDG_SESSION_TYPE=wayland` и `[general.environment]`.

## Оболочка и layer-shell

Пространства имён layer-поверхностей (`namespace`), по ним композитор решает, как с ними
обращаться (анимации, размытие, исключение из снимков):
`syndesktop-wallpaper` (background), `syndesktop-panel` (top), `syndesktop-launcher`,
`syndesktop-popup`, `syndesktop-notification`, `syndesktop-osd` (overlay),
`syndesktop-lock` (через ext-session-lock).

## Сборка и запуск

```sh
cargo build --release
# вложенно в текущий сеанс (окно winit):
target/release/syndesktop --nested
# настоящий сеанс: выбрать «syndesktop» в менеджере входа (data/syndesktop.desktop)
```
