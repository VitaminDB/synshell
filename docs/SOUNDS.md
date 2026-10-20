# Звуки и звуковые темы

Оболочки synshell (рабочий стол, телефон, игровой режим) озвучивают события
по звуковой теме freedesktop (XDG Sound Theme). Настройки — «Параметры →
Звук → Звуки событий» (`[sound]`) и «Параметры → Игровой режим → Звуки»
(`[gameshell] sounds`, `sound_theme`, `sound_volume`).

## События

| Событие (имя по freedesktop) | Группа `[sound]` | Когда |
|---|---|---|
| `message-new-instant` | `notifications` | новое уведомление (не в «Не беспокоить») |
| `audio-volume-change` | `system` | щелчок новой громкостью |
| `screen-capture` | `system` | снимок экрана готов |
| `desktop-screen-lock`, `desktop-screen-unlock` | `system` | блокировка и разблокировка |
| `desktop-login` | `system` | вход в сеанс (один раз за сеанс) |
| `device-added`, `device-removed` | `devices` | USB-устройство подключено/отключено |
| `power-plug`, `power-unplug`, `battery-low` | `devices` | зарядка, низкий заряд (≤ 10 %) |
| `button-pressed` | `clicks` | щелчок по кнопке оболочки |
| `gameshell-navigate`, `-bump` | игровой режим | шаг по плиткам, упор в край |
| `gameshell-select`, `-back`, `-tab` | игровой режим | выбор, назад, вкладка |
| `gameshell-menu-open`, `-menu-close` | игровой режим | меню действий и быстрое меню |
| `gameshell-launch`, `-favorite`, `-startup` | игровой режим | запуск игры, избранное, вход в режим |

По умолчанию в обычных оболочках включены уведомления и системные звуки,
устройства и щелчки — выключены; в игровом режиме звучит всё (`[gameshell]
sounds`, громкость `sound_volume`), в нём щелчки — по его выключателю.

## Темы

Тема — каталог в `~/.local/share/sounds` или `/usr/share/sounds` с
`index.theme` и `stereo/<событие>.{oga,ogg,wav,flac}`. Не нашлось в теме —
ищется в наследуемых (`Inherits`), затем в `synshell` и `freedesktop`; имя
события укорачивается по дефисам (`message-new-instant` → `message`). Поэтому
готовые темы (у них нет событий игрового режима) в игровом режиме звучат
своими уведомлениями, а навигация — темой `synshell`.

Свои темы (пакет synshell, `data/sounds/themes`):

| Тема | Характер |
|---|---|
| `synshell` | мягкое стекло — по умолчанию |
| `synshell-console` | глубокие мягкие щелчки и воздушные переходы — по умолчанию в игровом режиме |
| `synshell-crystal` | яркая челеста, мерцающее стекло |
| `synshell-retro` | 8-битные сигналы |
| `synshell-wood` | маримба и калимба |

Готовые из пакетов Arch: `sound-theme-freedesktop`, `ocean-sound-theme`
(KDE), `deepin-sound-theme`, `sound-theme-elementary`, `cosmic-sound-theme`;
из AUR: `yaru-sound-theme`, `sound-theme-smooth`.

## Как сделаны свои темы

`data/sounds/gen/synth.py` синтезирует короткие звуки (ноты с огибающей:
стекло, мягкий тон, челеста, квадрат, маримба, калимба; шумовые щелчки и
«вжухи»; синтетическая реверберация) в ладу темы. Длинные мелодичные — вход,
выход, заставка игрового режима, запуск игры, «готово» — сгенерированы
нейросетью ACE-Step через synaptixd (`data/sounds/gen/ai.py`: несколько
вариантов, выбор лучшего по огибающей и спектру, обрезка, нормализация;
результат — `data/sounds/ai/<тема>/<событие>.flac`). YuE2 для коротких звуков
не подошёл: даёт ровный приглушённый пэд без атаки.

```sh
SYNAPTIXCTL=target/fast-release/synaptixctl python3 -I data/sounds/gen/ai.py [тема…]
python3 -I data/sounds/gen/synth.py data/sounds/themes
```

Файлы — FLAC 48 кГц стерео. Новая музыка из командной строки:
`SECONDS=6 LYRICS="[instrumental]" synaptixctl music acestep-v15-xl-turbo out.wav "описание"`.

## Код

| Где | Что |
|---|---|
| `syngui::audio::sfx` | декодирование (symphonia), микшер коротких звуков; поток вывода открывается при первом звуке и закрывается после 4 с тишины |
| `synshell-common/src/sound_theme.rs` | поиск тем и файлов событий |
| `synshell-ui/src/sounds.rs` | события, группы, кэш, наблюдатели (блокировка, батарея, вход) |
| `syngame-shell/src/nav.rs`, `overlay.rs` | звуки игрового режима |
| `synsettings/src/pages/audio.rs`, `game.rs` | выбор темы, прослушивание |
