---
name: project-templates
description: Ролик по шаблону проекта — слайд-шоу, Reels-слайды, нарезка под бит, интро, промо, «до и после» — подставить свои файлы, музыку и тексты в места шаблона; сохранить свой проект шаблоном. Брать для «сделай как в CapCut», «по шаблону», «слайд-шоу из фото».
---
# Шаблоны проектов

1. `template_project_list` — выбери подходящий (по описанию и числу мест).
2. `template_project_use {name, files: [пути по порядку], music, texts: {подпись: текст}}` — одним вызовом.
3. `slot_list` — какие места остались пустыми; `slot_fill {id|label, file|media, text, offset}` — заполнить.
4. Проверка — `frame_check` на 1–2 кадрах.

Свой проект шаблоном: места — `slot_add`/`slot_set`, затем `template_project_save {name, title, description, category}`.
