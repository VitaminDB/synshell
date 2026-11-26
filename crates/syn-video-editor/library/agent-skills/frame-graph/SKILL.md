---
name: frame-graph
description: Граф кадра клипа и блоки — эффекты нодами, ветвление и слияние (свечение, двойная экспозиция), объединить ноды в блок с вынесенными параметрами, сохранить в библиотеку, разгруппировать. Брать для «граф кадра», «сделай свой эффект из нескольких», «сохрани как блок».
---
# Граф кадра и блоки

- `graph_open id` — эффекты изображения клипа становятся цепочкой нод source → … → output; `graph_show id` — ноды, входы, параметры.
- Ноды: `graph_add {id, kind, params, after}` — color, blur, sharpen, vignette, lut, glow, upscale, merge (входы bg, fg, режим и opacity). Связи — `graph_link`/`graph_unlink`; правка — `graph_node`; убрать — `graph_remove`.
- Свечение: копия через blur → merge mode=screen с исходником.
- **Блок**: `block_group {id, nodes, name, description, expose, save: true}` — ноды в одну ноду-блок; `expose` — вынесенные параметры `[{key, label, node, param, min, max}]` (по умолчанию все числовые); `save` — в библиотеку («Эффекты → Блоки»). Значения — `block_set`; развернуть — `block_ungroup`.
- Готовые блоки: `block_list` → `block_apply {ids, name, values}`.
- Вернуть к списку эффектов: `graph_close` (с ветвлением — `force: true`, граф отбросится).

Проверь `frame_check` до и после.
