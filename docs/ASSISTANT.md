# Ассистент

Помощник на системных нейросетях: отвечает, управляет системой, работает с файлами и терминалом, ищет в интернете,
создаёт медиа. Включается в «Параметры → Ассистент» (`[assistant] enabled`).

```
 окно «Ассистент» (synassistant) ─┐                  ┌─ synaptixd: generate с инструментами
 быстрая панель оболочки (Super+A)┤─ synassist ──────┤  (шаблон модели, разбор хода synaptix::facade::chat)
 synassistant --cli               ┘  чаты, агент,     └─ инструменты: система, окна, файлы, bash, web, медиа
                                     инструменты
```

## Составные части

| Что | Где |
|---|---|
| Ядро: чаты, агентный цикл, инструменты (без интерфейса) | `crates/synassist` |
| Окно с чатами | `crates/synassistant` |
| Быстрая панель оболочки | `crates/synshell-ui/src/assistant.rs` (команда оболочки `assistant`, по умолчанию Super+A) |
| Страница настроек | `crates/synsettings/src/pages/assistant.rs` |
| Разбор хода модели (текст, размышления, вызовы) | `synaptix/crates/synaptix/src/facade/chat/` (перенесено из synthos) |

`synassist` — общий с synthos крейт: генерация спрятана за `agent::Backend` (`DaemonBackend` — служба synaptixd;
synthos может подставить генерацию в процессе), язык сообщений — типы клиента `synaptixd` (`Message`, `ToolCall`,
`Event`, `Params`).

## Служба

`generate` принимает `tools` (схемы функций OpenAI) и историю с `name`/`reasoning`/`tool_calls`; служба рендерит
chat-шаблон модели с инструментами (`apply_chat_template_reasoning`), разбирает поток `facade::chat::StreamParser`
(ChatML `<tool_call>`, Gemma 4 по спецтокенам, канальный протокол Muse Glimmer, Llama-JSON, `<think>`) и шлёт события
`token` (ответ), `thinking`, `tool_text` (вызов пишется); в итоге — `text`, `thinking`, `tool_calls`. Генерация
останавливается, как только модель дописала вызов.

## Агентный цикл

`Agent::run_turn`: системный промпт (`prompt.rs`, стабилен в течение дня — префикс-KV службы) + история чата → модель →
вызовы инструментов → результаты (до 12 тыс. символов) → снова модель, до 24 шагов. Если шаги кончились, а модель всё
ещё зовёт инструменты, — последний ход без инструментов с просьбой подвести итог. Инструменты с `Danger::Confirm`
(запись и удаление файлов, bash, закрытие окон, сон/перезагрузка/выключение) спрашивают пользователя, если не включено
`[assistant] auto_approve`; отказ модель получает как результат. Группы выключаются в `[assistant] disabled_tools`.

Общее с synthos (`synassist::tools`):

- `args` — аргументы вызова: обрезка пробелов, приведение к типам схемы (`"True"` → `true`, `"30"` → `30`), имя
  вызова к ключу инструмента (`bash.run`, `functions.web` → `bash`, `web`);
- `bash` — `bash -lc` в своей группе процессов, stdout/stderr читаются одновременно с потолком 256 КБ (лишнее
  вычитывается и отбрасывается), `timeout_sec` (по умолчанию 600 с) и «Стоп» убивают группу целиком.

**Сжатие контекста** (`compact.rs`, порт из synthos): служба возвращает `prompt_tokens` и `context` (окно KV-сессии
модели). Когда промпт занял 85% окна, старая часть разговора (до последнего вопроса — между ходами; цепочка
инструментов текущего хода, кроме двух последних сообщений, — внутри хода) сворачивается моделью в сводку: сообщения
помечаются `compacted`, перед ними встаёт `Role::Summary`, а сводка уходит модели в системный промпт. Пары
«вызов — результат» не разрываются. Выключается `[assistant] autocompact = false`.

**Субагент** (`subagent`, выключается в `disabled_tools`): модель поручает узкую подзадачу вложенному агенту с тем же
бэкендом и набором инструментов (или подмножеством `tools`), без вложенных субагентов, до 16 шагов. Пользователь
подтверждает только запуск, шаги внутри выполняются без вопросов; в историю возвращается лишь итог.

| Инструмент | Что |
|---|---|
| `system` | open_app (по ярлыкам .desktop), open_settings, volume, brightness, wifi, bluetooth, notify, reminder, info, lock/suspend/reboot/poweroff |
| `windows` | список окон synwm, focus/minimize/maximize/close |
| `files` | list, read, search, write, append, mkdir, move, delete, open |
| `bash` | команда с `timeout_sec`, код выхода, stdout/stderr (общий код с synthos) |
| `subagent` | вложенный агент для подзадачи, возвращает итог |
| `web` | поиск (DuckDuckGo → Bing) и чтение страниц (Readability → markdown), из synthos |
| `media` | speak (OmniVoice через службу, wav в `~/.local/share/synassist/media`); image/video/music — когда служба научится |

Чаты — `~/.local/share/synassist/chats/<id>.json`, общие для окна и панели («Открыть чат» в панели открывает его в окне).

Проверка без окна: `synassistant --cli "вопрос"` (подтверждение — в терминале).
