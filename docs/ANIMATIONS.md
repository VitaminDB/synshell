# Анимации: что делает каждый пункт `[animations]`

«Параметры → Оформление → Анимации» пишут `[animations]` в `config.toml`. Ниже — где каждый пункт применяется
(проверено 2026-10-07; раньше частицы, размытие, волна и закрытие окон в настройках были, а в коде — нет).

| Пункт | Где действует |
|---|---|
| `enabled` | всё: synwm (`anim::duration` = 0), оболочка (`shell_ms`/`group_ms`/`effect` = 0/false), **syngui** — `syngui::animation::set_enabled`: переходы MSS не запускаются, `Animation` (пружины, твины, `Presence`), конечные ключевые кадры и переходные виджеты со своим временем (`effective_dt`: Carousel, Fisheye, Checkbox, ScrollView-плавная прокрутка, …) завершаются за кадр; бесконечные индикаторы, курсор, инерция прокрутки — идут. Программам synwm передаёт `SYNGUI_ANIMATIONS`, оболочка и «Параметры» переключают на лету |
| `speed` | synwm и оболочка — длительности; syngui — `set_speed` / `SYNGUI_ANIMATION_SPEED` (переходы MSS, `Animation`, ключевые кадры) |
| `window_open`, `minimize`, `workspace_switch`, `layout_changes` | synwm (`wm/ops.rs`, `render.rs`) |
| `window_close` | synwm: при закрытии/скрытии видимого окна — снимок последнего кадра (`Backend::thumbnail`), гаснет на месте (`zoom` — уменьшаясь, `slide` — вниз, `fade`) — `Wm::closing`, `render::closing_elements` |
| `reduce_motion` | группы оболочки вдвое короче, без частиц и волн (`effect`); synwm — вдвое короче и растворение вместо выезда/масштаба (`anim::motion_style`) |
| `shell` | всплывающие окна, меню, уведомления, OSD (`anim::ms`); стиль `.popup-morph` |
| `theme_change` | перетекание темы и обоев (`theme_ms`) |
| `home` | домашний экран: перелистывание (`Carousel::slide_duration_ms`), точки столов, значки, пульс запуска; экран блокировки |
| `menu`, `shade`, `pages` | «Пуск», шторка, «Недавние» и режимы окон (`group_ms`; synwm — `pages`) |
| `dock` | увеличение значков (Fisheye), эффект наведения, прыжок при запуске, выезд, подпись, «внимание» |
| `particles` | искры наведения и запуска дока (`dock.rs`; при выключении — без всплеска: у излучателя по умолчанию `burst = 30`) |
| `ripple` | волна от точки касания/нажатия — synwm (`input.rs::start_ripple`, `render::ripple_elements`), во всех программах |
| `blur` | **не реализовано**: оболочка рисует меню отдельными поверхностями, `backdrop-filter` syngui видит только свою поверхность — нужно размытие в композиторе |

Стили выключенных групп гасит `synshell-ui/src/theme.rs::animation_overrides` (`transition: none; animation: none`
для селекторов группы) — новые анимированные классы оболочки нужно вносить туда же.
