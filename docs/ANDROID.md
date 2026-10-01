# syndroid — Android-приложения в synshell

syndroid запускает Android (LineageOS) в контейнере рядом с synshell: каждое Android-приложение — обычное
окно synwm, установленные приложения — в «Программах», уведомления — в шторке. Это своя замена Waydroid на
Rust: без Python, LXC и gbinder. Используются готовые образы LineageOS из OTA-каналов Waydroid (system +
vendor MAINLINE), с хостом они общаются по протоколу Waydroid (binder-сервисы `lineageos.waydroid.*`,
свойства `waydroid.*`). Код Waydroid (GPL-3.0) не переносится — только совместимость по протоколу;
лицензия syndroid — как у всего synshell (MIT OR Apache-2.0).

Платформенная часть (ядро с `PID_NS`, сборка образа, GPU) — `arch-mobile-port/docs/16-android.md`.

## Состав
Один крейт `crates/syndroid`, бинарники:

| Бинарник | Кто запускает | Что делает |
|---|---|---|
| `syndroidd` | systemd (`syndroid.service`), root | контейнер, образы, сеть, binder; API по unix-сокету `/run/syndroid/syndroidd.sock` (JSON построчно, `api.rs`) |
| `syndroid` | пользователь | окно управления (syngui) **и** CLI: `syndroid status|start|stop|restart|app …|image …|prop …|shell|logcat` |
| сессионная часть | synwm-сессия (`syndroid session`) | сокеты Wayland/PipeWire пользователя → контейнер, .desktop-файлы, уведомления, буфер обмена |

Права (по `SO_PEERCRED`): читать состояние — все; запустить Android для своего сеанса, открыть/остановить
приложения, показать Android, остановить/перезапустить/заморозить — владелец сеанса, для которого запущен
Android; остальное (образы, настройки, установка/удаление приложений, данные) — root и группы wheel/android.
Опасные действия (удаление образов, сброс данных, удаление Android) окно подтверждает отдельно.

## Окно управления (`syndroid` без аргументов)
Полный контроль над Android из synshell, адаптивная раскладка (телефон/десктоп), разделы:

- **Состояние** — запущен/остановлен/заморожен, версия образов, аптайм, потребление памяти/CPU контейнера;
  кнопки «Запустить», «Остановить», «Перезапустить», «Заморозить»; лог (logcat) с фильтром.
- **Приложения** — список установленных (имя, значок, пакет, версия, системное или нет): открыть, остановить,
  удалить, очистить данные, разрешения (через `pm`/`appops`), показывать ли в «Программах»; установка APK
  из файла (synfiles / диалог выбора).
- **Образы** — установленные system/vendor (канал, версия, дата, размер); проверка обновлений по OTA-каналу,
  загрузка с прогрессом и проверкой sha256, переключение VANILLA/GAPPS, несколько наборов образов
  (например, LineageOS 18.1 и 20) с выбором активного; локальный образ из файла; откат на предыдущий.
- **Настройки** — режим окон (каждое приложение отдельным окном / весь Android одним окном), разрешение/DPI,
  ориентация, сеть (вкл/выкл, NAT), звук, общие папки хоста (Загрузки, Изображения → `/sdcard/…`),
  автозапуск с сессией, свойства `waydroid.*`/`persist.*` (редактор), расширенное: GPU (аппаратный/программный),
  выделение памяти.
- **Данные** — размер `/data`, резервная копия/восстановление, полный сброс Android.

## Окно управления (`gui.rs`, как сделано)
Каркас — как у synpkg: тема из конфига synshell, стили `styles/syndroid.mss` (основа — synpkg), на рабочем
столе навигация слева и подробности приложения справа, уже 720 px — нижняя навигация и подробности отдельным
экраном. Все запросы к демону — в фоновых потоках, состояние опрашивается раз в 1,5 с.
- «Обзор»: экземпляр, состояние, образы, версия, время работы, память контейнера (cgroup), кнопки по
  состоянию (Запустить / Показать Android / Перезапустить / Заморозить / Остановить), ход заданий, ошибки.
- «Приложения»: список из ярлыков экземпляра (мост держит их в согласии с Android — замороженный Android не
  будится), открыть/остановить/очистить данные/удалить (с подтверждением), установка APK по пути
  (демон кладёт файл в `/data/local/tmp` экземпляра и вызывает `pm install -r -g`). Список перечитывается,
  когда меняется каталог ярлыков экземпляра.
- «Образы»: загрузка LineageOS или LineageOS + GApps (это разные экземпляры), проверка обновлений, экземпляры
  с размером данных и наборами (использовать/удалить набор, выбрать экземпляр, сбросить данные, удалить
  Android целиком; пока он запущен — недоступно), импорт из файлов.
- «Настройки»: автозапуск, режим окон, сеть, плотность экрана, свойства Android (имя/значение), OTA-каналы;
  черновик сохраняется кнопкой.
- «Журнал»: logcat (свежие сверху) с фильтром.

**Грабли syngui.** `Reactive`, отдающий несколько виджетов, не раскладывает их — всегда один `Column`.
Представление с полями ввода не должно зависеть от часто меняющихся сигналов (статус обновляется каждые
1,5 с; черновик настроек при каждом символе): иначе поле пересоздаётся и теряет фокус — отдельные сигналы
(`running`, `cfg_rev`) и чтение черновика без отслеживания.

**Замороженный Android.** Без видимых окон Android через несколько минут «засыпает» и просит хост заморозить
его (`IHardware.suspend` → `cgroup.freeze`), как Waydroid. Для окна это рабочее состояние; любое действие с
Android (запуск приложения, APK, logcat) демон начинает с разморозки.

## Демон: устройство
1. **Образы** (`/var/lib/syndroid/images/<набор>/{system,vendor}.img`): загрузка по OTA JSON (`url`, `id` = sha256),
   распаковка zip, проверка; активный набор — `active` в `/var/lib/syndroid/config.toml`.
2. **Корень**: `system.img` и `vendor.img` (loop ro, autoclear) под overlay: свои файлы → слой платформы → образ,
   верхний слой — изменения Android (`overlay_rw/<набор>/`); `waydroid.prop` — bind из `/run/syndroid`;
   `/data` — `/var/lib/syndroid/data` (bind).
3. **Контейнер** — свой рантайм вместо LXC (`container.rs`): стартер `syndroidd __container` в новых
   NEWNS/NEWUTS/NEWNET/NEWPID (IPC не отделяется — в ядре нет `IPC_NS`), после «go» от демона (cgroup, veth) —
   NEWCGROUP и fork PID 1, `/dev` на tmpfs с нужными узлами (binder из binderfs, `kgsl-3d0`, `dri/*`, `ashmem`,
   `fuse`, `tun`, `uhid`, `sw_sync`, `dma_heap/*`), `pivot_root`, сокращение capabilities, seccomp, cgroup v2,
   `exec /init`. Заморозка — `cgroup.freeze`.
4. **Binder**: `binderfs` в `/dev/binderfs`, свои узлы `syndroid-{binder,vndbinder,hwbinder}` → `/dev/{binder,…}`
   контейнера. Клиент/сервер на `rsbinder` (Android 13): хост регистрирует в servicemanager контейнера
   `waydroidusermonitor`, `waydroidclipboard`, `waydroidhardware`, `waydroidnotifications` и вызывает
   `waydroidplatform` (список приложений, запуск, установка, `settings`, свойства).
5. **Сеть**: veth + мост `syndroid0` + NAT через `iptables-legacy` (в GKI нет nf_tables) + DHCP/DNS (свой
   dnsmasq).
6. **Свойства**: `waydroid.prop` — gralloc `gbm`, EGL `mesa`, Vulkan `freedreno`, сокеты Wayland/PulseAudio,
   размер окна и DPI, `persist.waydroid.multi_windows`.

## Слой платформы
Устройство-специфичное syndroid не знает: его ставит сборка ОС устройства (arch-mobile-port —
`tools/install-android.sh`).
- `/usr/share/syndroid/overlay/{system,vendor}` — файлы поверх образов (средний слой overlay: свои
  `overlay/` → платформа → образ), например turnip с KGSL под Android;
- `/etc/syndroid/*.prop` — свойства Android (Mesa: `vendor.mesa.loader.driver.override=zink` и т. п.);
  `[properties]` из `config.toml` — поверх них.

## CLI (этап 1)
`syndroid status | start | stop | restart | freeze | unfreeze`, `syndroid image list | check | fetch |
import SYSTEM VENDOR [ИМЯ] | use ИМЯ | remove ИМЯ`, `sudo syndroid shell [команда]`, `sudo syndroid logcat`,
`syndroid config`. Вход в контейнер — помощник `__exec` (setns + fork: команда оказывается и в пространстве PID
контейнера). Изменения — участники групп wheel/android и root; `start` — только для своего сеанса.

## Этапы
1. ✅ (2026-10-01) Демон: образы (загрузка/импорт/проверка sha256) + рантайм контейнера; Android 13 загружается
   до `sys.boot_completed` (~30 с на Redmi K50 Ultra), GPU — zink на turnip-KGSL, сеть по DHCP.
2. ✅ (2026-10-01) Окна: `syndroid show` (весь Android одним окном), `syndroid app list|launch|stop`; запуск
   приложения сам поднимает Android и ждёт загрузки; автозапуск с сеансом (`autostart`, `syndroid session`).

## Экземпляры Android и меню
- **Экземпляр** — линейка образов без даты сборки: `lineage-20.0-20260927-VANILLA` → `lineage-20.0-VANILLA`
  («LineageOS 20.0»; GAPPS — «LineageOS 20.0 · GApps»). Обновление образа той же линейки — тот же экземпляр;
  разные версии/варианты — разные экземпляры. У каждого свой `/data` (`/var/lib/syndroid/data/<экземпляр>`;
  прежний общий `/data` переносится в экземпляр, который запустится первым) и свои ярлыки.
- `syndroid app launch|show --instance <экз>`: если запущен другой Android — переключить на свежий набор этого
  экземпляра и перезапустить.
- **Меню** (`start_menu.rs`, телефон и рабочий стол): под поиском чипы «Linux» и «Android» (значок Android;
  несколько экземпляров — по чипу на каждый с названием). «Все приложения» и страницы телефона показывают
  выбранный источник; в смешанных местах (закреплённые, рекомендуемые) у значка Android-приложения — зелёная
  метка Android; в поиске — подпись «Android · LineageOS 20.0». Классическое меню (`launcher.rs`) — свои
  разделы «Android» по экземплярам, в «Все приложения» и категориях — только Linux. Страница приложений
  домашнего экрана телефона — только Linux.

## Окна Android (как это устроено)
- Окна создаёт hwcomposer образа (Wayland-клиент synwm; работает под uid владельца сеанса — сокет доступен).
  Что показывать, он берёт из свойства `waydroid.active_apps`: `Waydroid` — весь Android, имя пакета — окно
  этого приложения (`app_id` = `waydroid.<пакет>`, заголовок — имя приложения), `none` — ничего. Окна
  пересоздаются на следующем кадре: для «показать всё» syndroid дёргает шторку (`cmd statusbar`), как Waydroid.
- Ввод — каналы `/dev/input/wl_*_events`, их hwcomposer создаёт сам, поэтому /dev контейнера — tmpfs 1777.
- Сокеты сеанса — **ссылками** на каталог `XDG_RUNTIME_DIR` пользователя, привязанный целиком
  (`/run/xdg-host`, 0700 — внутри доступен только uid владельца сеанса): привязка файла сокета держит старый
  инод, и после перезапуска композитора hwcomposer не мог переподключиться — Android терял экран. Теперь
  hwcomposer падает при обрыве, init его перезапускает, и он подключается к новому сокету; Android при этом
  перезапускает system_server (~20 с) — `app launch` ждёт сервис пакетов до 40 с.
- Размер «экрана» Android — из первого configure окна (`waydroid.display_width/height`, `display_scale`).
- `multi_windows = true` — каждое приложение своим окном freeform: размеры окон задаёт Android, а не композитор,
  поэтому на телефоне по умолчанию выключено (приложение на весь экран Android в своём окне synwm).
  Свойство `persist.waydroid.multi_windows` действует со следующей загрузки Android (init.rc включает freeform
  при старте); демон выравнивает его с настройкой после каждой загрузки.
- Значки приложений Android сам кладёт в `/data/icons/<пакет>.png` — на хосте это
  `/var/lib/syndroid/data/icons/` (для ярлыков в «Программах», этап 3).
3. ✅ (2026-10-01) Binder: приложения Android в «Программах» (имена, значки, категории; обновляются при
   установке/удалении), уведомления Android в шторке synshell (нажатие — обратно в Android), выключение/
   перезагрузка/сон из Android управляют контейнером. Буфер обмена делает сам hwcomposer образа (Wayland
   data-device) — отдельного сервиса не нужно.

## Мост binder (`bridge.rs`)
- Отдельный процесс `syndroidd __bridge` на каждый запуск контейнера, от имени владельца сеанса (env: HOME,
  XDG_RUNTIME_DIR, DBUS_SESSION_BUS_ADDRESS); демон убивает его вместе с контейнером. Отдельный процесс —
  потому что `rsbinder::ProcessState` один на процесс и привязан к одному binder-устройству.
- Открывает `/dev/binderfs/syndroid-binder` прямо с хоста (binder не смотрит на пространства имён).
  servicemanager Android 13 — вручную (`checkService` = 2, `addService` = 3 по handle 0): `rsbinder::hub`
  на Linux собирает только протокол Android 16. Токен интерфейса — как у libbinder (strict mode, work source,
  'SYST', имя). Посылки `lineageos.waydroid.*` — тоже вручную: `AppInfo`, `Action`, `ImageData` — старые
  Java-Parcelable без префикса размера, AIDL-кодогенерация для них не годится.
- **Сервисы хоста надо зарегистрировать до старта system_server**: `WayDroidService.onStart` берёт
  `waydroidusermonitor`, `waydroidhardware`, `waydroidnotifications` один раз; если какого-то нет — исключение,
  и ни пакеты, ни уведомления не подключаются до следующей загрузки. Поэтому мост стартует сразу с
  контейнером и ждёт только servicemanager (`checkService("manager")`).
- Ярлыки — **отдельно от программ Linux** (у Waydroid они смешаны, и это путает):
  `~/.local/share/syndroid/applications/<экземпляр>/<пакет>.desktop` — вне `$XDG_DATA_DIRS/applications`,
  другие среды их не видят; synshell читает их отдельно (`xdg::android_apps_dir`, поле
  `DesktopEntry::android`). Ключи `X-Syndroid-Instance`, `X-Syndroid-Title`, маркер `X-Syndroid=true`;
  `Exec=syndroid app launch --instance <экз> <пакет>`, `StartupWMClass=waydroid.<пакет>` (окно получает
  значок); плюс «Весь Android» (`syndroid show --instance <экз>`, `Icon=syndroid`,
  `StartupWMClass=Waydroid` — у окна всего Android hwcomposer образа жёстко ставит app_id и заголовок
  «Waydroid»; по этому ключу оболочка показывает наше имя и значок, подпись «Android · <экземпляр>» —
  `xdg::window_subtitle`). Значок пакета — `data/icons/syndroid.svg` (ставится в hicolor). Значки — копии из `/data/icons` в
  `~/.local/share/syndroid/icons/<экземпляр>/` (Android перезаписывает их на каждой загрузке; копируется только
  целый PNG — с IEND).
- Уведомления → `org.freedesktop.Notifications` (hint `desktop-entry` = `waydroid.<пакет>`); служебные
  уведомления пакетов `android` и `com.android.systemui` (USB, зарядка) не пересылаются.
4. ✅ (2026-10-01) Окно управления (`syndroid` без аргументов, ярлык «Управление Android» — системная
   программа Linux, не Android): «Обзор», «Приложения», «Образы», «Настройки», «Журнал» (см. ниже).
5. ✅ (2026-10-01) Звук, поворот, общие папки, датчики (см. ниже).

## Звук, поворот, общие папки
- **Звук**: аудио-HAL образа — клиент PulseAudio (`/run/xdg/pulse/native` → ссылка в каталог сеанса), у нас —
  PipeWire-pulse; поток Android в PipeWire зовётся «Waydroid» (зашито в HAL), идёт на устройство по умолчанию.
- **Поворот**: датчики Android не нужны — поворачивает synshell, окно Android получает новый configure,
  hwcomposer меняет размер «экрана», приложение перестраивается на ходу.
- **Общие папки** (`shared_folders`, по умолчанию включено): папки пользователя из `~/.config/user-dirs.dirs`
  → `/data/media/0/{Download,Pictures,Music,Movies,Documents}`. Права: Android открывает хранилище через
  группу `media_rw` (1023), файлы создаёт от uid MediaProvider; idmapped mounts нет (ядро 5.10) — поэтому
  ACL на каталогах папок: `g:1023:rwx` и по умолчанию `g:1023`, `u:<владелец сеанса>` (файлы Android
  пользователь читает и меняет). FUSE MediaProvider читает не /data/media, а `/mnt/pass_through/0/emulated`
  (привязка vold без MS_REC — вложенные монтирования туда не попадают), поэтому после загрузки демон
  привязывает папки и туда (`android::share_folders`; корень Android — rshared, MediaProvider их видит).
  Удалять файлы общих папок в обход Android можно, но MediaProvider помнит запись: создать файл с тем же
  именем из Android сразу не выйдет (ENOENT) до пересканирования.
- **Датчики** (`sensors.rs`, `hwbinder.rs`): процесс `syndroidd __sensors` (root, на каждый запуск контейнера)
  отдаёт Android HIDL-сервис `android.hardware.sensors@1.0::ISensors/default` по hwbinder контейнера — его
  объявляет VINTF vendor-образа (как у Waydroid с `waydroid-sensord`); заглушку образа выключает
  `waydroid.stub_sensors_hal=0`. HIDL в rsbinder нет — свой минимальный hwbinder на ioctl: scatter-gather буферы
  (`BC_TRANSACTION_SG`/`BC_REPLY_SG`) для `hidl_vec`/`hidl_string`, методы IBase для hwservicemanager,
  регистрация `IServiceManager::add`. Раскладки HIDL: `SensorInfo` 112 байт, `Event` 80 байт.
  Данные — от **источника платформы** `/usr/lib/syndroid/sensors-source` (строки `A x y z`, `G`, `M`, `L lux`,
  `P 0|1`; `--list` — набор): syndroid запускает его только для включённых Android датчиков; нет источника —
  остаётся заглушка. Источник для Qualcomm SSC — arch-mobile-port `sensors/syndroid-ssc-sensors.c`.
  Грабли: SensorService вызывает `poll` и синхронно при инициализации (поток `system-server-init`) — блок «до
  первого события» вешал system_server (сторож через 60 с), поэтому `poll` ждёт не дольше 0,5 с. Автоповорот
  Android демон после загрузки выключает (`accelerometer_rotation=0`): поворачивает synshell, иначе содержимое
  повернулось бы ещё раз внутри окна.
- Свой бинарник для помощников — `/proc/self/exe`: после обновления файла на диске `current_exe()` указывает на
  удалённый путь, и `__exec`/`__bridge` не запускались.
6. ✅ (2026-10-01) Сборка образа телефона: всё syndroid попадает в образ без ручных шагов (программы, юнит,
   автозапуск, ярлык — `install-synshell.sh`; dnsmasq, turnip под Android и свойства устройства —
   arch-mobile-port). Проверено чистой сборкой rootfs рядом с sysroot (`ROOTFS_DIR=…`). Образы LineageOS в
   rootfs не входят (около 2,3 ГБ) — скачиваются из окна «Управление Android». Отдельная группа не нужна:
   свой Android запускает любой пользователь, управление образами — администраторы (wheel).
