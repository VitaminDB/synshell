---
name: motion-graphics
description: Моушн-графика — ключи и кривые, пресеты движения, анимированные титры, фигуры, маски, матовое стекло, режимы наложения и привязка слоёв. Брать для «анимируй», «влёт титра», «плашка», «нижняя треть», «наезд», «стрелка».
---
# Моушн-графика

- **Ключи**: `key_set {id, param, at, value, ease}` для transform (x, y, scale, rotation, opacity), mask, color, blur, vignette, sharpen, lut, style титра и фигуры. Сглаживание: linear, ease, in, out, back, elastic, bounce, hold, bezier. Убрать — `key_remove`, `key_clear`; посмотреть — `key_list`.
- **Пресеты движения**: `motion_list` → `motion_apply {preset, ids, duration}` — вход, движение по клипу, выход (ставят ключи transform).
- **Титры**: `title_add {text, at, duration, style}`; анимация в style: `by` (all|char|word|line), `in`/`out` (fade|slide|pop|scatter|type|none), `anim`, `stagger`, `ease`, `font` (`font_list` — шрифты).
- **Фигуры**: `shape_add kind=rect|ellipse|triangle|star|line|arrow` + style (цвет, обводка, скругление).
- **Маска**: `effect_add kind=mask {shape: rect|ellipse, x, y, width, height, radius, feather, invert}`.
- **Матовое стекло** (glassmorphism): `effect_add kind=glass {blur, tint, tint_opacity, border_opacity, highlight, shadow, saturation, brightness}`.
- **Наложение и привязка**: `clip_set blend` (normal, multiply, screen, overlay, add, darken, lighten, soft_light, hard_light, difference), `clip_set parent` — потомок двигается за родителем.

Проверь 1–2 кадра `frame_check` (титр читается, не уходит за край).
