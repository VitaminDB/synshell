<div align="center">

# synlink

**Связь устройств synshell — телефона и компьютера — по USB и Wi-Fi: экран, ввод, звук, файлы, уведомления, общий буфер обмена и `ssh phone` без знания адресов.**

[English](README.md) · [Сайт](https://alexeyev.dev/ru/synshell/syn-link/) · [AUR: synshell-link-bin](https://aur.archlinux.org/packages/synshell-link-bin) · [AUR: synshell-link-git](https://aur.archlinux.org/packages/synshell-link-git)

</div>

## Возможности

- **Находит устройства сама** — по кабелю USB телефона (сеть NCM) и в одной сети Wi-Fi; если доступны оба пути, остаётся один сеанс — по USB (подключили кабель — сеанс переезжает на него).
- **Защищённое соединение** — QUIC поверх UDP с TLS 1.3, ключ ed25519 у каждого устройства и закреплённые отпечатки; независимые потоки, так что копирование файлов не тормозит экран.
- **Спаривание** — по USB компьютер соглашается сразу, а телефон спрашивает одно подтверждение; по Wi-Fi на обоих экранах показывается код из 6 цифр. Машина с `discoverable = false` в Wi-Fi не видна.
- **Экран** — трансляция с управлением в [synlink-view](../syn-link-view/README.ru.md): передаются только изменившиеся места, крупные изменения идут видео H.264/HEVC с аппаратного кодера той стороны (V4L2, VAAPI, NVENC), остальное досылается без потерь, когда экран замер.
- **Звук** — во время трансляции звук устройства играет здесь, а здешний микрофон виден там как «Микрофон <имя>».
- **Ввод** — мышь, касания, клавиши и текст на другом устройстве.
- **Файлы** — файловая система устройства монтируется через FUSE и видна в Проводнике в «Устройствах»; файл, брошенный на значок связи на панели, копируется в «Загрузки» устройства.
- **Уведомления** — уведомления одного устройства показываются на другом.
- **Общий буфер обмена** — текст, скопированный на одном устройстве, вставляется на другом; секреты из «Паролей» стираются везде, когда их стёрли там, откуда копировали.
- **ssh** — `ssh phone` или `ssh <имя>` внутри соединения, по кабелю и по Wi-Fi; без системного sshd демон поднимает sshd пользователя на `127.0.0.1`.
- **Сети Wi-Fi** — после спаривания телефон предлагает перенести сети Wi-Fi компьютера (NetworkManager или iwd); пароли передаются только после согласия на телефоне.
- **Разработчикам и ИИ-агентам** — командная строка `synlink` и MCP-сервер для Claude Code: снимки экрана, касания, жесты, текст, команды, журналы, уведомления, файлы и запись экрана.

Телефон, пока подключён кабелем, показывает в шторке «Кабель USB подключён · оболочки соединены» и карточку компьютера; настройки — «Параметры → Связь с устройствами» (`[link]` в `config.toml`, меняются на лету). Как это устроено внутри — [docs/LINK.md](../../docs/LINK.md).

## Установка

synlink входит в пакет `synshell-link` (демон synlink и окно [synlink-view](../syn-link-view/README.ru.md)) — x86_64 и aarch64:

```sh
yay -S synshell-link-bin      # готовые бинарники
yay -S synshell-link-git      # сборка из исходников
```

Зависит от `synshell-core`. Необязательно: `openssh` (ssh на связанные устройства), `fuse3` (файлы связанных устройств в Проводнике). Входит в метапакет `synshell`: `yay -S synshell-bin` ставит весь рабочий стол. **Из исходников:** `cargo build --release -p syn-link` в репозитории [synshell](../../README.md).

Демон запускается сеансом (`/etc/xdg/autostart/synlink.desktop`), по одному на пользователя.

## Командная строка

```sh
synlink status                 # эта машина, кабель, устройства
synlink pair accept phone      # ответить на запрос спаривания из терминала
synlink shot phone -o a.png    # снимок экрана (PNG)
synlink tap phone 540 1200     # касание (пиксели снимка или доли 0..1)
synlink swipe phone 0.9 0.5 0.1 0.5 300
synlink type phone 'привет'    # текст в поле с фокусом
synlink key phone ctrl+a       # сочетание клавиш
synlink action phone back      # действие synwm: back, shell home, shell shade…
synlink exec phone -- uname -a # команда (вывод потоком, код выхода)
synlink shell phone            # терминал
synlink logs phone -f -u synshell
synlink view phone             # окно трансляции
synlink audio phone on         # звук трансляции (off — выключить; --no-mic — без микрофона)
synlink mount phone            # файлы (обычно смонтированы сами)
ssh phone                      # ssh внутри соединения
```

Устройство — имя, начало id, `phone`/`desktop`/`laptop` (если такое одно) или `local` — эта машина.

**Claude Code:** `claude mcp add synlink -- synlink mcp` — инструменты `devices`, `screenshot`, `tap`, `long_press`, `swipe`, `scroll`, `type_text`, `key`, `action`, `windows`, `exec`, `logs`, `notifications`, `read_file`, `write_file`, `record`.

## Настройки

```toml
[link]
enabled = true        # демон в сеансе
name = ""             # имя для других (пусто — модель устройства)
discoverable = true   # видна в Wi-Fi для спаривания
notifications = true  # пересылать и показывать уведомления
auto_mount = true     # файлы соединённых устройств — в Проводник
clipboard = true      # общий буфер обмена
port = 47471          # UDP QUIC; поиск — port − 1
```

## Часть synshell

synlink — одно из приложений [synshell](../../README.md): окружения рабочего стола для Wayland на Rust для компьютеров и телефонов, на UI-фреймворке [syngui](https://github.com/VitaminDB/syngui).

## Лицензия

GPL-3.0-only
