# Док, значки запуска, разделы и папки

Любая панель может стать **доком** — как Dock в macOS, Latte Dock или Cairo-Dock:
значки приложений в полосе, значок под курсором плавно вырастает над ней, соседи
расступаются. Индикаторы открытых окон, «прыжки» при запуске, частицы, 3D-наклон
и вращение значков, 3D-полка с отражениями, подпись над значком, автоскрытие и
умное скрытие.

И на доке, и на обычной панели есть:

- **значки запуска** (`app`) — быстрый запуск приложения, он же кнопка его окон;
- **разделы** (`group`) — значок, по клику или наведению открывающий окно с
  набором приложений (например, «Разработка»);
- **папки** (`folder`) — содержимое каталога во всплывающем окне с переходом по
  подпапкам (Загрузки, Корзина, любой путь).

Всё это добавляется и переставляется мышью в **режиме редактирования**.

## Как включить

- Правый клик по пустому месту панели → «Сделать доком» (и обратно).
- «Параметры системы» → «Панели и доки» → «Вид: Док», там же все настройки дока и
  кнопка «Добавить док» (готовый док с меню, приложениями, окнами, Загрузками и
  Корзиной).
- Вручную в `~/.config/synshell/config.toml`:

```toml
[[panel]]
mode = "dock"            # panel | dock
edge = "bottom"          # top | bottom | left | right
floating = true          # отступ от края экрана
exclusive = false        # резервировать место под полосу
autohide = false         # прятать всегда, показывать у края экрана
autohide_delay = 1500    # через сколько мс прятаться после ухода курсора (отрыва пальца)
applets = [
    { type = "launcher" },
    { type = "separator" },
    { type = "app", app = "org.kde.dolphin" },
    { type = "app", app = "firefox" },
    { type = "group", name = "Разработка", view = "fan", items = ["org.kde.konsole", "code", "org.kde.kate"] },
    { type = "taskbar" },                       # работающие приложения без значка
    { type = "separator" },
    { type = "folder", path = "xdg:DOWNLOAD", name = "Загрузки" },
    { type = "folder", path = "trash:" },
]

[panel.dock]
icon_size = 48           # px без увеличения
zoom = 1.7               # во сколько раз растёт значок под курсором (1 — без увеличения)
zoom_range = 2.5         # радиус увеличения, в значках
style = "glass"          # glass | shelf (3D-полка) | flat | neon | none
labels = true            # имя над значком при наведении
indicator = "dot"        # dot | dots (по точке на окно) | line | glow | none
hover_effect = "lift"    # lift | tilt (3D-наклон) | spin (3D-вращение) | glow | none
hover_particles = "none" # sparkle | magic | embers | bubbles | hearts | snow | trail | none
launch_animation = "bounce" # bounce | pulse | spin | none
launch_particles = "stars"  # stars | sparkle | confetti | fireworks | magic | poof | none
intellihide = false      # прятать, когда полосу перекрывает окно
```

### Автоскрытие

Спрятанный док (`autohide`, а также `intellihide`, когда его перекрывает окно)
показывается при подводе курсора к краю, а на телефоне — свайпом от этого края:
жест композитора (`[gestures] edge_bottom` и другие с действием `shell …`)
сначала показывает спрятанный док или панель у своего края и только следующим
разом выполняет само действие («домой», шторка) — как навигационная панель
Android в полноэкранном режиме. Спустя `autohide_delay` мс без курсора и пальца
док прячется снова (пока открыто его окно — стек, меню — не прячется). Команда
`synwm msg action "shell reveal-panels [bottom|top|left|right]"` показывает
спрятанные панели и доки из сочетания клавиш или кнопки.

## Значки, разделы, папки

| Тип | Параметры |
|---|---|
| `app` | `app` — id .desktop (`firefox`, `org.kde.dolphin`); `name`, `icon` (имя из темы значков, путь или один глиф Material Icons), `command` — своя команда; `label = true` — подпись на обычной панели |
| `group` | `name`, `icon` (без него — сетка 2×2 из значков раздела, как папка на iOS), `items` — id приложений и пути, `view` — `grid`, `list`, `fan` (веер над доком снизу, как «Стопки» macOS), `open` — `click` или `hover` |
| `folder` | `path` — путь (`~/Проекты`), `xdg:DOWNLOAD` (`DOCUMENTS`, `PICTURES`, `MUSIC`, `VIDEOS`, `DESKTOP`) или `trash:`; `name`, `icon`, `view`, `open` |

Клик по значку приложения: нет окон — запуск; есть — поднять окно; окно уже в
фокусе — свернуть (одно) или перейти к следующему. Средняя кнопка — новое окно,
колесо — перебор окон, правый клик — меню (окна, «Новое окно», «Открепить»,
«Закрыть все окна», «Настроить…»). У работающего приложения без значка в меню есть
«Закрепить».

В папке: клик по каталогу — переход внутрь (стрелка назад — наверх), по файлу —
открыть программой по умолчанию; внизу — «Открыть в файловом менеджере», у Корзины —
«Очистить».

## Режим редактирования

Правый клик по пустому месту панели или дока → «Изменить…» (или по значку →
«Изменить док»), команда `synwm msg action "shell edit-dock"` / `"shell edit-panel N"`.

- значки покачиваются; **перетаскивание** меняет порядок;
- **×** на значке — убрать;
- **клик** по значку — настроить: подпись, значок, команда; у раздела — название,
  значок, вид, способ открытия и приложения; у папки — путь;
- **+** в конце — окно «Добавить»: *Приложение* (поиск, клик — значок на панели),
  *Раздел* (форма с выбором приложений), *Папка* (готовые: Домашняя, Загрузки,
  Документы, Изображения, Рабочий стол, Корзина — или свой путь), *Апплет*;
- **✓** — закончить.

Каждая правка сразу пишется в `config.toml` (комментарии сохраняются) и
применяется.

## Внешний вид через MSS

Док — обычные элементы syngui с классами, всё переопределяется в
`~/.config/synshell/theme.mss` или `shell.mss` темы:

| Класс | Что это |
|---|---|
| `.dock-root.dock-<край>.dock-style-<стиль>.dock-ind-<индикатор>.dock-hover-<эффект>.dock-launch-<анимация>` | корень; `.dock-hidden` — спрятан, `.dock-editing` — режим правки, `.dock-floating` |
| `.dock-slide` | полоса целиком — её сдвигает автоскрытие (`translate-y`, `opacity`) |
| `.dock-items` | полоса значков (`Fisheye`): фон, рамка, тени, `magnification`, `magnification-range`, `magnification-falloff` (`cosine`, `gaussian`, `linear`), `magnification-speed`, `background-rotate-x/y` + `background-perspective` — 3D-наклон только подложки |
| `.dock-item` (+ `-app`, `-group`, `-folder`, `-launcher`; `-running`, `-active`, `-minimized`, `-urgent`, `-launching`, `-open`) | квадрат значка; `:hover` — эффекты наведения |
| `.dock-icon` | картинка значка (например, `box-reflect` для отражения) |
| `.dock-dots` > `.dock-dot` (`.dock-dot-active`) | индикатор окон |
| `.dock-fx-hover-<пресет>`, `.dock-fx-launch-<пресет>` | частицы (`particle-*`) при наведении и запуске |
| `.dock-label` > `.dock-label-text` | подпись над значком |
| `.dock-separator-line`, `.group-preview`, `.stack-*`, `.edit-*` | разделитель, значок раздела, всплывающие стеки, режим правки |

Примеры:

```css
/* значок под курсором вырастает вдвое, радиус — 4 значка */
.dock-items { magnification: 2; magnification-range: 4; }

/* своя 3D-полка */
.dock-style-shelf .dock-items {
    background: linear-gradient(180deg, #ffffff55, #ffffff10);
    background-rotate-x: 60deg;
    background-perspective: 360px;
}

/* значок переворачивается при наведении */
.dock-item:hover { rotate-y: 180deg; transition: rotate-y 600ms ease-out-back; }

/* свой всплеск частиц при запуске */
.dock-fx-launch-stars {
    particle-preset: fireworks;
    particle-burst: 80;
    particle-colors: "#ff5e5e #ffd24d #5ee0ff";
}

/* магический след за курсором над доком */
.dock-fx-hover { particle-preset: magic; particle-hover-rate: 30; particle-emitter: pointer; }
```

Анимации — MSS `@keyframes` (`dock-bounce-up`, `dock-pulse`, `dock-spin`,
`dock-attention`, `edit-wiggle`) и переходы (`transition`). 3D-свойства, частицы и
«рыбий глаз» — возможности syngui (см. его документацию: `06-styling.md`,
`13-effects.md`).

## Как устроено

- `synshell-ui/src/dock.rs` — поверхность дока (layer-shell `syndesktop-dock`,
  на всю длину края и с запасом над полосой под увеличение и подпись), ряд значков,
  подпись, автоскрытие/умное скрытие, **область ввода** (`syngui_layer::set_input_region`):
  клики над полосой проходят к окнам;
- `launchers.rs` — значки запуска, разделы и папки (общие для панели и дока),
  сопоставление окон с приложениями, всплывающие стеки;
- `edit.rs` — режим редактирования, меню, окно «Добавить», формы;
- `synshell-common/src/config_edit.rs` — правки `config.toml` из оболочки
  через `toml_edit`.
