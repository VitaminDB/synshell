# Темы оформления

![Встроенные темы](themes.jpg)

Тема задаёт палитру (тёмный и/или светлый вариант), обои без картинки, цвета
заголовков окон и собственный MSS для оболочки и «Параметров системы».
Выбирается на странице «Темы» в настройках или в `config.toml`:

```toml
[appearance]
theme = "synthwave"
color_scheme = "dark"   # какой вариант темы взять
accent = ""             # пусто — акцент темы
```

Встроенные: `nord`, `catppuccin`, `tokyo-night`, `gruvbox`, `rose-pine`,
`everforest`, `dracula`, `kanagawa`, `solarized`, `synthwave`, `aurora`,
`sakura`, `brutal`, `terminal`. Список с источниками:
`synsettings --list-themes`.

## Что откуда берётся

Слои палитры (каждый следующий перекрывает предыдущий):

1. стандартная тёмная/светлая палитра synshell;
2. вариант темы `[dark]` или `[light]` (если у темы один вариант, он
   используется при любой `color_scheme`);
3. `appearance.accent`, если не пустой;
4. `[appearance.colors]`.

Слои стиля оболочки: переменные палитры → переменные темы → встроенный
`shell.mss` → `shell.mss` темы → фон рабочего стола → `~/.config/synshell/theme.mss`.
«Параметры системы» устроены так же, но со своим `settings.mss`.

Обои темы видны, когда в `[wallpaper]` пусты `path` и `color`. Заголовки окон
берут цвета темы при `active_color = "theme"` / `inactive_color = "theme"`
(по умолчанию).

При выборе темы в настройках записываются её рекомендации (скругление,
прозрачность панелей, толщина панелей, рамки окон, шрифт — если установлен), сбрасываются свой
акцент, `[appearance.colors]` и цвета обоев. Вручную в `config.toml` можно
поменять одну строку `theme` — тогда остальные значения остаются как были.

## Своя тема

Каталог `~/.config/synshell/themes/<имя>/` (или
`/usr/share/synshell/themes/<имя>/` для всех пользователей). Тема с именем
встроенной заменяет её. Композитор и оболочка следят за файлами активной темы
и применяют правки на лету — удобно подбирать стиль, держа файл открытым.

`theme.toml`:

```toml
name = "Моя тема"
description = "Одна строка для галереи."
author = "я"

# Рекомендации — записываются в config.toml при выборе темы в настройках.
[appearance]      # corner_radius, panel_opacity, font, font_size, icon_theme, cursor_theme
corner_radius = 12
panel_opacity = 0.9

[decorations]     # title_height, font_size, title_align, buttons, corner_radius, shadow, shadow_size, shadow_opacity
title_align = "left"

[panel]           # size, defloated_size — во все панели, кроме доков
size = 44                 # толщина плавающей панели
defloated_size = 38       # толщина прилипшей к краю (defloat = "maximized" | "touch")

[dark]            # и/или [light]
bg = "#1e1e2e"            # фон, панели
surface = "#262637"       # меню, окна, карточки
surface_alt = "#313244"   # поля, кнопки
fg = "#cdd6f4"
muted = "#a6adc8"
border = "#45475a"
accent = "#cba6f7"
accent_fg = "#1e1e2e"     # необязательно — иначе по контрасту
danger = "#f38ba8"
success = "#a6e3a1"
warning = "#f9e2af"
wallpaper = "linear-gradient(160deg, #11111b 0%, #1e1e2e 50%, #5b4a7a 100%)"
wallpaper_color = "#1e1e2e"   # сплошной цвет под обоями
titlebar = "surface"          # accent | surface | surface_alt | bg | #rrggbb
titlebar_inactive = "bg"
shadow = "#00000080"          # --shadow
scrim = "#00000060"           # --scrim

[dark.vars]       # свои переменные: --glow и т. п.; можно перекрыть и встроенные (--panel-bg)
glow = "#cba6f766"
```

Все поля необязательны, кроме хотя бы одного варианта. Неизвестный ключ —
ошибка (опечатка не пройдёт молча); тема с ошибкой пропускается, в логе будет
причина.

`shell.mss` — правила поверх встроенных (`crates/syndesktop-shell/styles/shell.mss`,
там же список классов: `.panel`, `.ws-active`, `.task-active`, `.popup-card`,
`.notif`, `.launcher-row-selected`, `.clock-time`, `.lock-avatar`, …). Доступны
переменные палитры (`--bg --surface --surface-alt --panel-bg --menu-bg --fg
--muted --border --accent --accent-fg --accent-soft --hover --pressed --danger
--success --warning --radius --radius-sm --font-size --shadow --scrim`) и свои
из `vars`. Работают градиенты (`linear-`, `radial-`, `conic-gradient`),
`box-shadow` (несколько, `inset`), `glow`, `outline-*`, `text-shadow`,
`text-transform`, `letter-spacing`, переходы. Примеры — во встроенных темах
(`crates/synshell-common/themes/`): неон в `synthwave`, жёсткие тени в
`brutal`, стекло в `aurora`.

Перетекания: карточка всплывающего окна у прижатой панели получает класс
`.popup-flow-top|bottom|left|right` — встроенный стиль снимает у неё рамку и
скругления со стороны панели и рисует вогнутые «ушки» (`flow-edge`,
`flow-radius: var(--radius)`, `flow-color: var(--panel-bg)`); тема может
переопределить радиус или цвет. `.popup-morph` задаёт пружину перетекания
размера (`transition: size 320ms spring(420, 40)`). При смене темы цвета
всех элементов перетекают за `[animations]`-длительность (≈450 мс × `speed`),
если включено `theme_change`.

Прилипшая к краю панель (`[[panel]] defloat`) получает класс
`.panel-defloated` вместо `.panel-floating`; встроенный стиль убирает у неё
рамку, скругления и тень (`.panel.panel-defloated`). Тема может вернуть,
например, линию по внутреннему краю:
`.panel.panel-defloated.panel-top { border-bottom: 1px solid var(--border); }`.

## Цвета программ

При `[appearance] app_colors = true` (по умолчанию; «Внешний вид → Программы»)
оболочка при каждой смене темы, схемы, акцента или цветов палитры
перекрашивает программы под итоговую палитру:

| Что | Файл | Кто читает |
|---|---|---|
| GTK 3/4, тема Breeze | `~/.config/gtk-{3,4}.0/colors.css` (`*_breeze`) | GTK-тема Breeze |
| GTK 4 libadwaita, GTK 3 adw-gtk3 | блок `/* synshell: … */` в `~/.config/gtk-{3,4}.0/gtk.css` | libadwaita (`--window-bg-color` …), adw-gtk3 |
| тёмная/светлая схема | `settings.ini` (`gtk-application-prefer-dark-theme`), `gsettings … color-scheme` | GTK, порталы |
| Qt/KDE | `[Colors:*]`, `[WM]` в `~/.config/kdeglobals`, схема `~/.local/share/color-schemes/Syndesktop.colors` | Qt с `QT_QPA_PLATFORMTHEME=kde`, kded6 |
| GIMP 3 | блок в `~/.config/GIMP/<версия>/gimp.css` | стандартная тема GIMP |

Роли цветов: фон окна — `surface` в тёмной схеме и `bg` в светлой, фон
содержимого (списки, поля, холсты) — наоборот; кнопки — `surface_alt`/`surface`,
выделение — акцент, заголовки — цвета заголовков окон темы.

Файлы переписываются только при изменении; свои строки в `gtk.css`,
`gimp.css` и остальные группы `kdeglobals` сохраняются, перед первой правкой
`kdeglobals` сохраняется копия `kdeglobals.synshell-backup`. Программы Qt/KDE
перекрашиваются сразу (сигнал `KGlobalSettings.notifyChange`), GTK — у новых
окон (открытые — при наличии `colorreload-gtk-module`), GIMP — после
перезапуска. У GIMP должна быть выбрана тема «Default» (любой вариант).
