//! title: До и после
//! category: reels
//! description: вертикальный ролик 8 с — «до», шторка, «после», подписи ДО / ПОСЛЕ и итог
cmd("project.settings", #{width: 1080, height: 1920, fps: 30});
let a = cmd("slot.add", #{kind: "media", at: 0, duration: 3.5, label: "До", motion: true}).data.id;
let b = cmd("slot.add", #{kind: "media", at: 3.5, duration: 4.5, label: "После", motion: true}).data.id;
cmd("transition.set", #{id: b, kind: "wipe_right", duration: 0.8});
cmd("effect.add", #{ids: [a], kind: "color", params: #{saturation: -0.35}});
cmd("slot.add", #{kind: "text", at: 0.2, duration: 3.0, label: "Подпись «до»", text: "ДО", style: #{position: "top", size: 120, box: true, "in": "pop"}});
cmd("slot.add", #{kind: "text", at: 4.0, duration: 4.0, label: "Подпись «после»", text: "ПОСЛЕ", style: #{position: "top", size: 120, box: true, "in": "pop"}});
cmd("slot.add", #{kind: "text", at: 5.5, duration: 2.5, label: "Итог", text: "Результат за 30 дней", style: #{position: "bottom", size: 64, "in": "slide", by: "word"}, track: "Итог"});
let m = cmd("slot.add", #{kind: "music", at: 0, duration: 8, label: "Музыка"});
cmd("effect.add", #{ids: [m.data.id], kind: "audio_fade_out", params: #{duration: 1.0}});
