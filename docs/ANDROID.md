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
| `syndroidd` | systemd (`syndroid.service`), root | контейнер, образы, сеть, binder; API по unix-сокету `/run/syndroid.sock` |
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
   распаковка zip, проверка; активный набор — ссылка `current`.
2. **Корень**: `system.img` (loop, ro) как `/`, `vendor.img` (loop, ro) в `/vendor`; поверх — overlay для
   своих файлов (Mesa с KGSL, `waydroid.prop`); `/data` — `/var/lib/syndroid/data` (bind).
3. **Контейнер** — свой рантайм вместо LXC: `clone(NEWPID|NEWNS|NEWNET|NEWUTS|NEWCGROUP)` (IPC не отделяется —
   в ядре нет `IPC_NS`), `/dev` на tmpfs с нужными узлами (binder из binderfs, `kgsl-3d0`, `dri/*`, `ashmem`,
   `fuse`, `tun`, `uhid`, `sw_sync`, `dma_heap/*`), `pivot_root`, сокращение capabilities, seccomp, cgroup v2,
   `exec /init`. Заморозка — `cgroup.freeze`.
4. **Binder**: `binderfs` в `/dev/binderfs`, свои узлы `syndroid-{binder,vndbinder,hwbinder}` → `/dev/{binder,…}`
   контейнера. Клиент/сервер на `rsbinder` (Android 13): хост регистрирует в servicemanager контейнера
   `waydroidusermonitor`, `waydroidclipboard`, `waydroidhardware`, `waydroidnotifications` и вызывает
   `waydroidplatform` (список приложений, запуск, установка, `settings`, свойства).
5. **Сеть**: veth + мост `syndroid0` + NAT через `iptables-legacy` (в GKI нет nf_tables) + DHCP/DNS (свой
   минимальный DHCP или dnsmasq).
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
2. Сессия: Wayland-сокет → окна Android в synwm (сначала одним окном, затем multi-window).
3. Binder-сервисы: приложения в «Программах», запуск из оболочки, уведомления, буфер обмена.
4. Окно управления: Состояние → Приложения → Образы → Настройки → Данные.
5. GPU: Mesa под Android с KGSL (turnip + zink) в overlay vendor; звук через PipeWire-pulse; поворот и
   датчики; сеть и общие папки.
6. Сборка образа телефона: пакеты, юнит, группа `android`, (по желанию) предзагруженные образы.
