//! title: Интро канала
//! category: intro
//! description: заставка 16:9 на 6 с — три быстрых кадра через вспышку зума и карточка с названием и подзаголовком
cmd("project.settings", #{width: 1920, height: 1080, fps: 30});
let a = cmd("slot.add", #{kind: "media", at: 0, duration: 1.3, label: "Кадр 1", motion: true}).data.id;
let b = cmd("slot.add", #{kind: "media", at: 1.3, duration: 1.3, label: "Кадр 2", motion: true}).data.id;
let c = cmd("slot.add", #{kind: "media", at: 2.6, duration: 3.4, label: "Фон карточки", motion: true}).data.id;
cmd("transition.set", #{id: b, kind: "zoom", duration: 0.3});
cmd("transition.set", #{id: c, kind: "blur", duration: 0.5});
cmd("effect.add", #{ids: [c], kind: "blur", params: #{radius: 14}});
cmd("effect.add", #{ids: [c], kind: "color", params: #{brightness: -0.25}});
cmd("slot.add", #{kind: "text", at: 3.0, duration: 3.0, label: "Название", text: "НАЗВАНИЕ КАНАЛА", style: #{position: "center", size: 110, by: "char", "in": "scatter", anim: 0.7}});
cmd("slot.add", #{kind: "text", at: 3.6, duration: 2.4, label: "Подзаголовок", text: "подзаголовок", style: #{position: "bottom", size: 48, "in": "fade", color: "#ffffffcc"}, track: "Тексты 2"});
let m = cmd("slot.add", #{kind: "music", at: 0, duration: 6, label: "Музыка"});
cmd("effect.add", #{ids: [m.data.id], kind: "audio_fade_out", params: #{duration: 1.0}});
