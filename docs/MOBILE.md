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

## synmobile-shell (скелет)
`crates/synmobile-shell`: строка состояния (часы, батарея из `/sys/class/power_supply`), панель навигации
(назад = закрыть окно, домой, приложения), домашний экран — сетка `.desktop` (overlay-слой, прячется при
запуске). Общие сервисы — из `synshell-common` (конфиг, темы, `.desktop`, IPC). Стили: `styles/mobile.mss`
+ `~/.config/synshell/mobile.mss`. Дальше: экранная клавиатура (`zwp_input_method`/virtual-keyboard в
композиторе), шторка уведомлений, экран блокировки, жесты.

## Запуск на телефоне (Redmi K50 Ultra, см. arch-mobile-port)
```
# кросс-сборка с хоста (sysroot = rootfs телефона):
PROFILE=fast-release ~/Projects/2027/arch-mobile-port/tools/cross-rust.sh ~/Projects/2027/synshell -p synwm -p synmobile-shell
# на телефоне (Arch по USB):
LIBSEAT_BACKEND=noop SYNSHELL_DRM_DEVICE=/dev/dri/card0 XDG_RUNTIME_DIR=/run/weston synwm --tty --cpu
```
`Virtual-*` (writeback-коннектор) исключён через `platform.ignore_outputs`, иначе wlroots-подобная раздача
CRTC отдаёт панели не тот CRTC. VT в ядре нет — только `LIBSEAT_BACKEND=noop`.
