# syndesktop

Окружение рабочего стола для Wayland на Rust. Интерфейс написан на [syngui](../syngui).
Устроено как Plasma: отдельно композитор, оболочка и «Параметры системы».
Почти всё настраивается через `~/.config/syndesktop/config.toml`, изменения применяются на лету.

| Бинарь | Что делает |
|---|---|
| `syndesktop` | Композитор на smithay 0.7: DRM/KMS (несколько GPU, горячее подключение, DPMS) или вложенное окно. Серверные рамки, столы, раскладки floating/tile/columns/grid/monocle, прилипание к половинам экрана, обзор окон, Alt+Tab, правила окон, Xwayland, IPC. |
| `syndesktop-shell` | Оболочка на syngui (layer-shell): обои, панели с апплетами, меню запуска, уведомления (D-Bus), OSD, меню питания, переключатель окон, экран блокировки (ext-session-lock + PAM). |
| `syndesktop-settings` | «Параметры системы»: 18 страниц. Правит `config.toml` через toml_edit, комментарии сохраняются. |

Темы оформления — 14 встроенных (Nord, Catppuccin, Tokyo Night, Gruvbox, Rosé Pine, Everforest,
Dracula, Kanagawa, Solarized, Синтвейв, Аврора, Сакура, Необрутализм, Терминал) и свои на MSS:
[docs/THEMES.md](docs/THEMES.md).

![Темы](docs/themes.jpg)

Архитектура, IPC и контракт команд оболочки описаны в [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
Полный пример конфига с комментариями: [crates/syndesktop-common/default-config.toml](crates/syndesktop-common/default-config.toml).

## Сборка и запуск

```sh
cargo build --release
# вложенно в текущем сеансе (окно):
target/release/syndesktop --nested
# настоящий сеанс: makepkg -si, затем выбрать «syndesktop» в менеджере входа
```

## Управление

```sh
syndesktop msg windows | workspaces | outputs | layouts | events
syndesktop msg action "workspace 2"
syndesktop msg action "spawn firefox"
syndesktop msg window 5 minimize
syndesktop msg restart-shell   # перезапустить панели и меню, окна остаются
syndesktop msg restart         # перезапустить композитор (окна программ закроются)
```

Основные сочетания клавиш (любое можно переопределить в `[keybindings]`):

| Клавиши | Действие |
|---|---|
| Super+Return | терминал |
| Super+D / Alt+F1 | меню запуска |
| Alt+Space | строка запуска |
| Super+1…9 / Super+Shift+1…9 | перейти на стол / перенести окно на стол |
| Super+←/→/↑/↓ | прилепить окно к краю, развернуть, свернуть |
| Super+T | сменить раскладку |
| Super+Tab | обзор окон |
| Alt+Tab | переключение окон |
| Super+ЛКМ / Super+ПКМ | переместить / изменить размер окна |
| Print | снимок экрана |
| Super+Escape | заблокировать экран |
| Super+Shift+E | меню питания |

Логи пишутся в `~/.local/state/syndesktop/` (`syndesktop.log`, `shell.log`).
