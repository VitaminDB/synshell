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
  страницы, листаемые пальцем, — это рабочие столы (`[workspaces] count`): листание переключает стол в
  synwm, смена стола извне листает страницы, за край — по кругу при `wrap`; точки столов вверху (касание —
  переход). На первом столе — сводка (часы, процессор, память, батарея; `[mobile] resources_page`), на
  остальных — сетка приложений (`[mobile] home_apps`, `home_columns`) при `[wallpaper] desktop_icons`,
  иначе пусто. Удержание по значку — меню приложения. Свайп вверх — «Пуск», вниз — шторка.
  Обои `layout = "panorama"` едут вслед за пальцем, листающим столы, `"workspace"` — свои у каждого
  стола (сменяются на середине пути с растворением); кадр и обои настраиваются в «Параметры → Обои».
- **«Пуск»** (`[launcher] style = "win11"`, общий с десктопом): поиск, закреплённые страницами,
  «Все ›» → алфавитный список с icon rail, рекомендуемые; на телефоне — лист почти на весь экран.
  Вид на телефоне — `[mobile] launcher`: `pages` (все приложения значками по страницам) или `list`
  (как на десктопе); переключается кнопкой внизу «Пуска». Клавиатура — только по касанию поля поиска
  (фокус поля → `syngui_layer::set_virtual_keyboard_handler` → `synkeyboard show`).
- **Шторка** (`shell shade`): плитки (сеть, Bluetooth, не беспокоить, режим окон, фонарик, клавиатура,
  снимок, блокировка), яркость в процентах и нитах (`[[output]] max_nits`), громкость, уведомления.
- **Сеть** (плитка шторки, апплет `network`, `shell network`; `synshell-ui/src/netmenu.rs`): текущее
  подключение и Wi-Fi через `synsystem::wifi` (iwd или NetworkManager, `[wifi] backend`) — включить,
  список сетей с сигналом, подключение (пароль для защищённой новой сети — с экранной клавиатурой),
  отключить, забыть; «Параметры сети…» открывает страницу Wi-Fi параметров. Без беспроводного
  устройства (на телефоне — пока нет драйвера) окно так и говорит. Службы iwd и NetworkManager
  (`synsystem::wifi::services`): если ни одна не работает — кнопка «Запустить» (`systemctl enable --now`,
  от root или через pkexec), у работающей — «Остановить»; то же в «Параметрах → Wi-Fi → Служба». Без
  пакетов iwd/networkmanager — подсказка поставить их.
- **Док с автоскрытием**: свайп от его края (`edge_bottom` — «домой») сперва показывает спрятанный док,
  следующий — выполняет действие; прячется через `autohide_delay` мс (см. docs/DOCK.md).
- **Вибрация** (`[haptics]`, «Параметры → Вибрация»): `synshell_common::haptics` — вибромотор (input-устройство
  с force feedback, `qcom-hv-haptics`), поток держит его открытым; сила — FF_GAIN и уровень эффекта. Отклики: жесты
  от краёв (synwm; «Недавние» удержанием — сильнее, пока палец на экране), клавиши synkeyboard (при касании),
  удержание пальцем и переключатели во всех приложениях на syngui (`syngui::input::set_haptic_handler`),
  уведомления (кроме «Не беспокоить»). Процессы без слежения за конфигом перечитывают `[haptics]` сами при
  изменении config.toml. Проба с хоста: пример `synshell-common/examples/haptic.rs`.
- **Дата и время** («Параметры → Дата и время», `synsystem::time`): пояс, синхронизация по сети (NTP) и ручная
  установка часов — через systemd-timedated (D-Bus `org.freedesktop.timedate1`); «Часовой пояс автоматически»
  (`[time] auto_timezone`, по умолчанию на телефоне) — оболочка (`synshell_ui::datetime`) узнаёт пояс по внешнему
  IP (`curl`: ip-api.com, ipwho.is, ipapi.co) при входе, при появлении сети и раз в 3 часа. Часы оболочки
  перечитывают пояс сами (`tzset`). Тап по часам экрана ресурсов (и апплет часов) — календарь с быстрыми
  настройками; на телефоне окно по центру экрана. Без logind-сеанса polkit нужно правило для группы wheel
  (в arch-mobile-port — `50-synmobile-timedate.rules`).
- **«Недавние»** (`shell recents`): карточки окон, свайп вверх закрывает.
  Ещё из шторки: плитка «Недавние» и лента «Открытые приложения» (тап — перейти, «×» — закрыть, «Все» — «Недавние»).
- **Блокировка**: обложка с часами и уведомлениями, свайп вверх — PIN-панель или полная клавиатура «ABC».
- Всплывающие окна на телефоне — нижним листом во всю ширину, контекстные меню — у пальца.
- Команды для жестов композитора: `home`, `apps`, `shade`, `recents`, `back`, `keyboard`, `mode X`
  (`synwm msg action "shell home"`). Отладка без телефона: `SYNSHELL_FORM_FACTOR=phone synwm --nested
  --nested-size 400x880`; снимки поверхностей без композитора — `SYNGUI_LAYER_HEADLESS=406x904
  SYNGUI_LAYER_DUMP=каталог` + сценарий касаний `SYNGUI_LAYER_SCRIPT="1200 tdown syndesktop-home 60,400; …"`.
- Анимации — группы `[animations] home, menu, shade, dock, pages` (+ `reduce_motion`), каждую можно выключить.

Оболочка запускает и перезапускает демон `synkeyboard`. Окна оболочки на телефоне (`exclusive_zone = 0`)
встают над клавиатурой, даже если она появилась позже них: форк smithay раскладывает layer-поверхности с
exclusive zone первыми, а не в порядке появления (иначе «Пуск», открытый до клавиатуры, накрывал её).

## Режимы окон и жесты (synwm)
`[mobile] mode` (и плитка «Режим окон» в шторке, меню рабочего стола, `synwm msg action "mobile-mode free"`):

| Режим | Как выглядит | Устройство |
|---|---|---|
| `pages` (по умолчанию) | каждое приложение во весь экран, листание вбок вдоль нижнего края, «страница 0» — домашний экран | `Wm::mobile` (`wm/mobile.rs`): видна только страница (окно с диалогами), переход — сдвиг, как у столов; «свернуть» — домой |
| `free` | свободные окна на большом столе (`[mobile] desk` = 2x2 / 3x3 / infinite) | серверная рамка-ручка у всех окон (xdg-decoration → ServerSide), пан стола двумя пальцами — окна сдвигаются, камера в `MobileInfo` |

Жесты (`[gestures]`, `touch.rs`): касание в зоне края (`edge_size`) задерживается до `threshold` — свайп слева/справа
внутрь — `edge_left/right` («назад»: оверлею оболочки `shell back`, окну — клавиша `[mobile] back_key`, по умолчанию
XF86Back), снизу — `edge_bottom` («домой», `page home`; или `minimize-all` — просто свернуть все окна, как быстрый свайп в Android; спрятанный док «домой» не перехватывает), снизу до ~1/5 высоты экрана и удержать (~0,2 с, дрожание до 14 px не мешает) — `edge_bottom_hold` («Недавние», открываются по таймеру, не дожидаясь отпускания),
сверху — `edge_top` (шторка); не жест — касание воспроизводится приложению. Пальцем по рамке окна: кнопки
заголовка по тапу, заголовок тащит окно, край — размер. IPC: `Request::Mobile`, `Event::MobileChanged`.

## Экран ресурсов, «Программы», Wi-Fi, Bluetooth
- Первая страница домашнего экрана (`[mobile] resources_page`): процессор (кольцо, история, ядра с частотами и
  классом LITTLE/big/prime), память, Adreno, температуры, питание (без драйвера батареи — напряжение с АЦП PMIC),
  сеть; лента запущенных приложений с бейджами RAM/CPU (`synsystem::procs`).
- `synpkg` («Программы»): поиск в репозиториях и AUR, установка/удаление/обновление с живым логом. Pacman — от root
  или через pkexec; AUR — snapshot через curl (git не нужен), makepkg от `[packages] build_user` (на телефоне root —
  нужен обычный пользователь: `useradd -m builder`, `build_user = "builder"`). Сеть — Wi-Fi (или прокси хоста
  через `http_proxy`, если он задан).
- Пароль для pkexec спрашивает само приложение: `synsystem::polkit_agent` регистрирует процесс агентом polkit
  (субъект `unix-process`, как `pkttyagent --process`), окно ввода — `Prompter` приложения (в synpkg — поверх
  интерфейса). Сеансовый агент без logind не работает: polkitd ищет агента по процессу или logind-сеансу, а сеансы
  synlogin идут без logind. Проверка без интерфейса: `SYN_PASS=… cargo run -p synsystem --example polkit_agent`.
  Помощник PAM — через `/run/polkit/agent-helper.socket`, а если сокет молчит (нет pidfd в ядре < 6.5), то setuid
  `polkit-agent-helper-1`.
- Wi-Fi (`[wifi] backend`: iwd или NetworkManager) и Bluetooth (bluez) — страницы «Параметров» и окно «Сеть»
  оболочки. На телефоне Wi-Fi работает через NetworkManager (iwd на ядре GKI не стартует — нет AF_ALG):
  драйвер qca6490/cnss2, прошивка из раздела modem, `wlan-cnss-ready.service` — см. arch-mobile-port
  `docs/11-wifi.md`.

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
