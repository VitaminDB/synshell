---
name: fusion-templates
description: Шаблоны и переходы Fusion (.setting) и анимации Lottie — найти, наложить, заменить тексты, править параметры MacroOperator, переходы между клипами. Брать для «наложи шаблон», «красивая нижняя треть», «переход как в Resolve», «титр из библиотеки».
---
# Шаблоны Fusion и Lottie

- **Найти**: `template_list {query}` (поиск по библиотеке; разделы — титры, нижние трети, переходы, графика, Lottie).
- **Наложить**: `template_apply {name|path, text, at, duration, track}`; свои строки — в `text` через `\n`. Lottie (.json/.lottie) встаёт клипом медиа поверх.
- **Параметры**: `template_params id` (с текущими значениями) → `template_set {id, param, value}` или `{id, params: {…}}`; сброс — `reset: true`. Анимировать параметр шаблона — `key_set param=tpl.<ключ>`.
- **Состав графа шаблона**: `template_inspect name` (тексты, инструменты, вынесенные входы).
- **Переходы**: `transition_list {query}` → `transition_set {id, kind: "fusion", template, duration, params}` (или `kind: "fusion:<имя>"`) на стыке двух клипов.
- Покрытие библиотеки (какие шаблоны исполняются полностью) — `template_coverage`.

После наложения — `frame_check` в середине шаблона: тексты целиком, без «Lorem ipsum».
