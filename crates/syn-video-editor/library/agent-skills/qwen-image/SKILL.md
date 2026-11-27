---
name: qwen-image
description: Картинки на Qwen-Image 2.1 (модель картинок по умолчанию в службе) — эталоны героев во весь рост и локаций для H3 Ref2VA, правка эталона (режим edit, base): переодеть, сменить эмоцию, вид сбоку и сзади, размеры, негатив, как писать промпт. Загружать перед ref-image, story-video (узел refs), обложками и кадрами.
---
# Qwen-Image 2.1 — эталоны и правка

## Промпт
- **По-английски**, конкретно: вид/возраст, телосложение, лицо, волосы/уши, одежда **сверху донизу** (штаны, обувь), особые предметы; затем стиль и свет.
- Один кадр, без текста: в конце `single clean film still, sharp detail, no text`; негатив против коллажа: `collage, storyboard, comic panels, grid of images, split screen, multiple frames, text, captions, watermark`.
- Стиль фильма — одними и теми же словами во всех эталонах (иначе герои из разных «мультфильмов»).

## Эталоны для видео (H3 Ref2VA)
- **Герой — во весь рост:** `Full-body character reference, entire figure from head to toe in frame (feet and shoes visible, empty space above the head and below the feet), standing neutral pose facing the camera, clean neutral background: …`; негатив `cropped body, cut-off legs, cut-off feet, close-up portrait, half-body shot`. Портрет по пояс → у H3 низ героя в каждой сцене разный.
- **Локация — пустая:** `Empty location establishing shot, no people or characters: …` (обстановка, предметы, палитра, свет).
- Один эталон — один герой или одна локация (не листы-разворотки).
- Размер: 1344×768 (16:9); для быстрой пробы 960×544.

## Правка эталона (режим edit — `ref-image` с `base`)
`recipe_run {name: "ref-image", open: false, params: {base: ["/путь/герой.png"], prompt: "…"}}` — Qwen-Image сохраняет лицо, стиль и пропорции, меняет то, что сказано:
- ракурсы: `the same character, full-body side view (left profile), same outfit, same proportions, same neutral background`; `… back view …` — отдельными картинками (для 1–3 главных героев, навык screenplay 4½);
- переодеть, эмоция, предмет в руках; локация — время суток, погода, разрушения;
- после правки проверь, что герой узнаваем и ноги в кадре.

## Проверка
Посмотри эталоны (просмотр/`frame_check`): обрезанные ноги, лишние герои, текст на картинке — перерисуй (другое зерно или уточни промпт).
