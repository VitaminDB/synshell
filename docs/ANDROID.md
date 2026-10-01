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

Доступ к сокету демона — группа `android` (+ root); опасные действия (смена/удаление образа, сброс данных)
дополнительно подтверждаются в окне.

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

## Окна Android (как это устроено)
- Окна создаёт hwcomposer образа (Wayland-клиент synwm; работает под uid владельца сеанса — сокет доступен).
  Что показывать, он берёт из свойства `waydroid.active_apps`: `Waydroid` — весь Android, имя пакета — окно
  этого приложения (`app_id` = `waydroid.<пакет>`, заголовок — имя приложения), `none` — ничего. Окна
  пересоздаются на следующем кадре: для «показать всё» syndroid дёргает шторку (`cmd statusbar`), как Waydroid.
- Ввод — каналы `/dev/input/wl_*_events`, их hwcomposer создаёт сам, поэтому /dev контейнера — tmpfs 1777.
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
- Ярлыки: `~/.local/share/applications/waydroid.<пакет>.desktop` (маркер `X-Syndroid=true`, чужие не трогаем),
  `Exec=syndroid app launch <пакет>`, `StartupWMClass=waydroid.<пакет>` (окно получает значок),
  значки — копии из `/data/icons` в `~/.local/share/syndroid/icons/` (Android перезаписывает их на каждой
  загрузке; копируется только целый PNG — с IEND).
- Уведомления → `org.freedesktop.Notifications` (hint `desktop-entry` = `waydroid.<пакет>`); служебные
  уведомления пакетов `android` и `com.android.systemui` (USB, зарядка) не пересылаются.
4. Окно управления: Состояние → Приложения → Образы → Настройки → Данные.
5. Звук через PipeWire-pulse; поворот и датчики; общие папки. (GPU — zink на turnip-KGSL из слоя платформы —
   сделан на этапе 1.)
6. Сборка образа телефона: пакеты, юнит, группа `android`, (по желанию) предзагруженные образы.
