# synshell на телефоне

Одна среда для десктопа и телефона, разделённая как Plasma / Plasma Mobile / KWin:
композитор общий, оболочки разные, форм-фактор — свойство времени выполнения.

## Как организовано
| Часть | Десктоп | Телефон | Где различие |
|---|---|---|---|
| Композитор `synwm` | GLES через GBM/EGL (`backend/tty.rs`) | pixman + dumb-буферы (`backend/kms_cpu.rs`) | бэкенд выбирается на старте: `[platform] renderer`, `--cpu/--gpu`, `SYNSHELL_RENDERER`, `auto` = есть ли EGL на устройстве |
| Политика окон | как настроено | monocle, без рамок и зазоров, decorations=client, без Xwayland | `Config::apply_form_factor` — только значения, которые пользователь не задал явно |
| Оболочка | `syndesktop-shell` | `synmobile-shell` | `general.shell` по умолчанию зависит от форм-фактора |
| Параметры, файлы, снимки | те же крейты | те же крейты | адаптивная раскладка syngui (в работе) |

Форм-фактор: `[platform] form_factor = "auto" | "desktop" | "phone"`; `auto` — телефон, если единственный
подключённый коннектор DRM — DSI-панель (`backend::probe_form_factor`). Детям композитора передаётся
`SYNSHELL_FORM_FACTOR`. Для отладки на десктопе: `SYNSHELL_FORM_FACTOR=phone synwm --nested`.

Cargo-features `synwm`: `pixman` (CPU-бэкенд, включён по умолчанию). GLES/GBM пока не отключаются —
на телефоне библиотеки есть в sysroot, а выбор идёт на старте.

## Почему CPU-композитор
На телефонах с Android-ядром (Qualcomm msm_drm/sde) нет GBM/EGL для KMS-устройства: Mesa не знает драйвер
`msm_drm`, а GPU доступен только через Vulkan turnip с бэкендом KGSL. Поэтому композитор рисует pixman'ом
в dumb-буфер, а клиенты рисуют на GPU и отдают dma-buf с **линейным** модификатором — компoзитор объявляет
linux-dmabuf только с линейными форматами (`PixmanRenderer::dmabuf_formats`) и импортирует буферы через mmap.
GPU-композитинг — следующий этап (свой рендерер на wgpu/Vulkan или zink+GBM поверх turnip).

## Состояние на телефоне (Redmi K50 Ultra, 2026-09-29)
- synwm с `--cpu` включает панель DSI-1 1220x2712 (smithay из форка `VitaminDB/smithay`, ветка `synshell`:
  alpha плана масштабируется под диапазон драйвера — sde отдаёт 0..255).
- Vulkan-клиенты (wgpu, turnip/KGSL) презентуют через `zwp_linux_dmabuf_v1` v3: turnip собран с
  `freedreno-kmds=kgsl,msm` (нужен libdrm в WSI) и патчем KHR_display — см. `arch-mobile-port/docs/08`.
- synmobile-shell стартует под synwm на телефоне; synkeyboard печатает в foot (буквы, заглавные, Ctrl+C,
  F-клавиши, EN/RU).

## synmobile-shell (скелет)
`crates/synmobile-shell`: строка состояния (часы, батарея из `/sys/class/power_supply`), панель навигации
(назад = закрыть окно, домой, приложения), домашний экран — сетка `.desktop` (overlay-слой, прячется при
запуске). Общие сервисы — из `synshell-common` (конфиг, темы, `.desktop`, IPC). Стили: `styles/mobile.mss`
+ `~/.config/synshell/mobile.mss`. Оболочка запускает и перезапускает демон `synkeyboard`; кнопка ⌨ на
панели навигации шлёт ему `toggle`. Дальше: шторка уведомлений, экран блокировки, жесты.

## synkeyboard — экранная клавиатура
`crates/synkeyboard`: отдельный демон на layer-shell (слой Top, снизу, exclusive zone = высота — окна
отодвигаются). Клавиши уходят композитору как evdev-коды через `zwp_virtual_keyboard_v1` (syngui-layer:
`virtual_keyboard_key/modifiers/group`, keymap собирается `xkbcommon` для всех языков сразу — `us,ru`,
язык переключается группой). Автопоказ — по `zwp_input_method_v2`: композитор активирует input-method,
когда сфокусированное окно включает text-input (foot, GTK, Qt); окна без text-input — кнопкой ⌨.

Раскладка (как Gboard): буквы EN/RU, страницы `?123` и `=\<` (символы — коды US-раскладки с Shift),
одноразовый Shift (двойной тап — Caps), нижний ряд `?123 ⌨ EN ␣ . ⏎`. Кнопка ⌨ раскрывает три
функциональных ряда: `Esc Tab Ctrl Alt ⌘ Ins Del PgUp PgDn ⌄`, `F1–F6 Home ↑ End`, `F7–F12 ← ↓ →`.
Ctrl/Alt/⌘ — залипающие на одну клавишу; ⌫, Del, стрелки, PgUp/PgDn — с автоповтором.

Важно: композитор (smithay) сообщает клиентам состояние модификаторов только по запросу `modifiers`
виртуальной клавиатуры — нажатие самой клавиши Shift/Ctrl его не меняет. Поэтому перед клавишей шлётся
маска (Shift=1, Control=4, Mod1=8, Mod4=64), после — нулевая.

CLI (сокет `$XDG_RUNTIME_DIR/synkeyboard.sock`, переопределяется `SYNKEYBOARD_SOCKET`):
```
synkeyboard show|hide|toggle|fn|lang
synkeyboard type "echo Hello"      # текст символами US-раскладки
synkeyboard key ctrl+shift+c       # сочетание; f1…f12, enter, tab, esc, стрелки, home/end/pgup/pgdn/del
```
Стили: `styles/keyboard.mss` + `~/.config/synshell/keyboard.mss` (переменные палитры из `[appearance]`).

## Запуск на телефоне (Redmi K50 Ultra, см. arch-mobile-port)
```
# кросс-сборка с хоста (sysroot = rootfs телефона):
PROFILE=fast-release ~/Projects/2027/arch-mobile-port/tools/cross-rust.sh ~/Projects/2027/synshell -p synwm -p synmobile-shell -p synkeyboard
# на телефоне (Arch по USB):
LIBSEAT_BACKEND=noop SYNSHELL_DRM_DEVICE=/dev/dri/card0 XDG_RUNTIME_DIR=/run/weston synwm --tty --cpu
```
`Virtual-*` (writeback-коннектор) исключён через `platform.ignore_outputs`, иначе wlroots-подобная раздача
CRTC отдаёт панели не тот CRTC. VT в ядре нет — только `LIBSEAT_BACKEND=noop`.
