---
name: video-story
description: Видеоистория / мультфильм по идее — сценарий, эталоны героев и локаций (Qwen-Image), голоса героев и все реплики заранее в Drama 3, съёмка сцен на MiniMax-H3 Ref2VA с синхронизацией губ под готовую озвучку (<Audio N> fully_copy), проверка, монтаж. Брать для «сделай мультик про…», «сгенерируй ролик по идее», «история из N сцен».
---
# Мультфильм / видеоистория: порядок работы

Загрузи навыки: `screenplay` (сюжет и раскадровка), `minimax-h3` (промпт видео), `drama3` (озвучка), `qwen-image` (эталоны). Видео — только MiniMax-H3 Ref2VA; LTX — только по прямой просьбе (навык `ltx`).

## Главный способ: речь заранее, H3 — губы, шумы и музыка
1. **Сценарий** (навык `screenplay`): сюжет, герои, локации, раскадровка по сценам. Для каждой сцены: **описание по-английски** (кадр, камера, свет, действие) и **реплики по-русски** с явным говорящим, 3–12 слов, с буквой ё и указаниями игры в [скобках] для Drama 3. Сцена — до 15 с (длина = озвучка + паузы).
2. **Эталоны** (навык `qwen-image`): каждый герой во весь рост и каждая локация пустая — `recipe_run {name: "ref-image", open: false, params: {prompt, style}}` по одному; путь из ответа запомни (это `<Picture k>`).
3. **Голоса героев** (навык `drama3`): `voice.cast {name, text}` — проба 8–15 с характерной речи с описанием голоса в [скобках] после первой фразы. Голос должен подходить герою (размер, возраст, характер). Сообщи пользователю, какие голоса получились (файлы в медиатеке).
4. **Реплики**: для каждой реплики сцены `voice.say {text, voice: "<герой>"}` — ответ даёт путь и длительность. Распознай сомнительные (`media_transcribe`); не совпало — другой `seed`.
5. **Дорожки сцены**: длина сцены = 0,4 с + реплики с паузами 0,4–0,6 с + 0,6 с в конце (не больше 15 с). На **каждого говорящего** сцены — `voice.track {seconds, lines: [{file, at}], name: "сцена-3-Грымза"}`: его реплики на своих секундах, вокруг тишина; дорожки сцены одинаковой длины.
6. **Съёмка**: `recipe_run {name: "ref-scenes", params: {refs: [все эталоны…, все дорожки всех сцен…], storyboard: <строки сцен>, style: "", chain: true, lang: "Russian", width: 960, height: 544}}`. Номера: картинки `<Picture k>` — по порядку картинок в `refs`, звук `<Audio k>` — по порядку звуковых файлов в `refs`. Каждая строка `storyboard` — промпт сцены **одной строкой** в формате Ref2VA из 6 полей (навык `minimax-h3`) с её `<Picture k>` и `<Audio k>` (роль `fully_copy`), реплики `<d>[Russian] …</d>` теми же словами, что в озвучке; `[CUT] ` в начале — при смене локации. Сцене достаются только упомянутые ею референсы.
7. **Проверка**: распознай речь 2–3 сцен (должна совпасть с репликами), посмотри кадры (`frame_check`: герои те же, губы двигаются у говорящего). Слабую сцену переснять: `ref-scenes` с одной строкой, `open: false`, затем заменить клип.
8. **Монтаж**: `skill_run cinematic_montage` (название, титры, цвет, звук) и экспорт.

Пример строки сцены (одной строкой; постановка — навык `minimax-h3`, «Постановка»):
`subject_definitions: <Picture 1> is Grymza, a big green swamp troll chef… <Picture 3> is the interior of Grymza's swamp hut… <Audio 1> is the exact dialogue track of Grymza. <Audio 2> is the exact dialogue track of Krya. summary: [reference generation] + [audio reuse] … retention_analysis: <Picture 1>: fully_preserved - Grymza's appearance. <Picture 3>: fully_preserved - the hut interior; the room layout, walls, window and stove stay exactly as in the reference. <Audio 1>: fully_copy - Grymza's speech, copied exactly and lip-synced. <Audio 2>: fully_copy - … detailed_description: 3D animated feature film, …; static locked-off camera, the room layout does not change. [Shot 1] Medium two-shot: <Picture 1> (S1) stands by the stove on the left…; <Picture 2> (S2) stands on the floor on the right, already in frame from the first frame. The two characters face each other in three-quarter profile and never look at the camera. (S1) glances at Krya and grumbles, <d>[Russian] Что за шум на болоте?!</d> Then (S2) turns to face Grymza, looks up at him and says excitedly to him, <d>[Russian] Грымза! Завтра конкурс супов!</d> Each character moves the mouth only while their own audio is audible; otherwise the mouth stays closed. overall_soundscape: crackling fire, bubbling pot. non_diegetic_music: soft playful orchestral score, low volume.`
Проверено 27.11.2026: речь в ролике дословно совпала с озвучкой, постановка по описанию.

## Быстрый способ без озвучки заранее
`recipe_run story-video` — всё одной командой (эталоны, план, сцены), но речь H3 придумывает сам: возможны акцент и чужие слова. Брать для роликов без важных реплик.

Модели генерации при пустом `model` / `image_model` берутся из настроек «Модели». Drama 3 и H3 вместе в видеокарту не помещаются — сначала вся озвучка (шаги 3–5), потом съёмка.
