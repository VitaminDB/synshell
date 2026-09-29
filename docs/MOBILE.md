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
| Системные данные | `synsystem` | `synsystem` | батарея, ядра и частоты, температуры, процессы, подсветка (ниты), GPU (KGSL) |

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

## synmobile-shell
`crates/synmobile-shell` — тонкая оболочка поверх общей библиотеки `crates/synshell-ui` (та же, что у
рабочего стола): панели, доки, меню, уведомления, блокировка — общие, форм-фактор — `ShellCtx::form_factor`.

- **Никаких встроенных строки состояния и навбара.** Панели и доки добавляет пользователь: удержание
  на рабочем столе (удержание = правая кнопка) → меню рабочего стола → «Добавить панель» / «Добавить док»
  (шаблоны: строка состояния с часами, сетью и батареей; док с меню и открытыми окнами). Панели хранят
  `[[panel]] form_factor = "phone"`; без ключа панель десктопная и на телефоне не показывается.
- **Домашний экран** — непрозрачная поверхность `syndesktop-home` под окнами (слой Bottom): обои и
  страницы, листаемые пальцем — сводка (часы, процессор, память, батарея; `[mobile] resources_page`) и
  сетка приложений (`[mobile] home_apps`, `home_columns`). Удержание по значку — меню приложения.
  Свайп вверх — «Пуск», вниз — шторка.
- **«Пуск»** (`[launcher] style = "win11"`, общий с десктопом): поиск, закреплённые страницами,
  «Все ›» → алфавитный список с icon rail, рекомендуемые; на телефоне — лист почти на весь экран.
- **Шторка** (`shell shade`): плитки (сеть, Bluetooth, не беспокоить, режим окон, фонарик, клавиатура,
  снимок, блокировка), яркость в процентах и нитах (`[[output]] max_nits`), громкость, уведомления.
- **«Недавние»** (`shell recents`): карточки окон, свайп вверх закрывает.
- **Блокировка**: обложка с часами и уведомлениями, свайп вверх — PIN-панель или полная клавиатура «ABC».
- Всплывающие окна на телефоне — нижним листом во всю ширину, контекстные меню — у пальца.
- Команды для жестов композитора: `home`, `apps`, `shade`, `recents`, `back`, `keyboard`, `mode X`
  (`synwm msg action "shell home"`). Отладка без телефона: `SYNSHELL_FORM_FACTOR=phone synwm --nested
  --nested-size 400x880`; снимки поверхностей без композитора — `SYNGUI_LAYER_HEADLESS=406x904
  SYNGUI_LAYER_DUMP=каталог` + сценарий касаний `SYNGUI_LAYER_SCRIPT="1200 tdown syndesktop-home 60,400; …"`.
- Анимации — группы `[animations] home, menu, shade, dock, pages` (+ `reduce_motion`), каждую можно выключить.

Оболочка запускает и перезапускает демон `synkeyboard`.

## Режимы окон и жесты (synwm)
`[mobile] mode` (и плитка «Режим окон» в шторке, меню рабочего стола, `synwm msg action "mobile-mode free"`):

| Режим | Как выглядит | Устройство |
|---|---|---|
| `pages` (по умолчанию) | каждое приложение во весь экран, листание вбок вдоль нижнего края, «страница 0» — домашний экран | `Wm::mobile` (`wm/mobile.rs`): видна только страница (окно с диалогами), переход — сдвиг, как у столов; «свернуть» — домой |
| `tiles` | окна друг под другом (раскладка `rows`) | граница «верхнее окно / остальные» тянется пальцем (`master_ratio`) |
| `free` | свободные окна на большом столе (`[mobile] desk` = 2x2 / 3x3 / infinite) | серверная рамка-ручка у всех окон (xdg-decoration → ServerSide), пан стола двумя пальцами — окна сдвигаются, камера в `MobileInfo` |

Жесты (`[gestures]`, `touch.rs`): касание в зоне края (`edge_size`) задерживается до `threshold` — свайп слева/справа
внутрь — `edge_left/right` («назад»: оверлею оболочки `shell back`, окну — клавиша `[mobile] back_key`, по умолчанию
XF86Back), снизу — `edge_bottom` («домой», `page home`), снизу с задержкой пальца — `edge_bottom_hold` («Недавние»),
сверху — `edge_top` (шторка); не жест — касание воспроизводится приложению. Пальцем по рамке окна: кнопки
заголовка по тапу, заголовок тащит окно, край — размер. IPC: `Request::Mobile`, `Event::MobileChanged`.

## Экран ресурсов, «Программы», Wi-Fi, Bluetooth
- Первая страница домашнего экрана (`[mobile] resources_page`): процессор (кольцо, история, ядра с частотами и
  классом LITTLE/big/prime), память, Adreno, температуры, питание (без драйвера батареи — напряжение с АЦП PMIC),
  сеть; лента запущенных приложений с бейджами RAM/CPU (`synsystem::procs`).
- `synpkg` («Программы»): поиск в репозиториях и AUR, установка/удаление/обновление с живым логом. Pacman — от root
  или через pkexec; AUR — snapshot через curl (git не нужен), makepkg от `[packages] build_user` (на телефоне root —
  нужен обычный пользователь: `useradd -m builder`, `build_user = "builder"`). Сеть телефона — через прокси хоста
  (`http_proxy`).
- Wi-Fi (`[wifi] backend`: iwd или NetworkManager) и Bluetooth (bluez) — страницы «Параметров»; на телефоне Wi-Fi
  ждёт драйвера и прошивки (arch-mobile-port).

## Автозапуск на телефоне
Через экран входа: `synlogin daemon -- --cpu` (см. ниже; на Redmi K50 Ultra — юнит устройства в
arch-mobile-port). Без экрана входа — `data/synshell-phone.service`: `synwm --tty --cpu` под `dbus-run-session`
с `LIBSEAT_BACKEND=noop`, сразу оболочка root.

## Экран входа synlogin
`synlogin daemon [-- аргументы synwm]` (root, юнит `crates/synlogin/data/synlogin.service`) по кругу:
1. композитор synwm с экраном входа вместо оболочки (`SYNSHELL_SHELL="synlogin greeter"`, `XDG_RUNTIME_DIR=
   /run/synlogin/greeter`): часы, карточки пользователей (UID 1000–59999 и root), пароль через PAM
   (`syndesktop-lock`, иначе `login`; пустой пароль — вход без пароля), «Новый пользователь» (логин, имя,
   пароль, администратор → `useradd -m -G video,input,audio,render[,wheel]`), перезагрузка и выключение,
   своя экранная клавиатура (`OnScreenKeyboard::stretch`), на телефоне показана сразу;
2. экран входа пишет имя в `/run/synlogin/request` и завершает композитор (`Action::Quit`);
3. сеанс: `dbus-run-session synwm` от имени пользователя (initgroups/setgid/setuid), `XDG_RUNTIME_DIR=
   /run/synlogin/session/UID` (не `/run/user`: его удаляет logind после выхода из ssh), узлы `/dev/dri`,
   `/dev/input`, `/dev/kgsl-3d0`, `/dev/dma_heap`, `/dev/snd`, яркость подсветки и светодиодов — во владение
   пользователю (logind-сеанса нет), после сеанса — обратно root и `pkill -u`;
4. «Выйти» в меню питания завершает композитор — снова экран входа; «Перезагрузка» и «Выключить» в сеансе
   (`SYNLOGIN_REQUEST`) композитор передаёт демону файлом и выходит — демон выполняет их от root.

Отладка без композитора: `SYNSHELL_FORM_FACTOR=phone SYNGUI_LAYER_HEADLESS=406x904 SYNGUI_LAYER_DUMP=dir
synlogin greeter`. `SYNLOGIN_SYNWM` подменяет бинарник композитора.

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
