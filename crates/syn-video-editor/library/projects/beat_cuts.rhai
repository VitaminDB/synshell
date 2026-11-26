//! title: Клип под бит
//! category: music
//! description: вертикальный клип 8 с — 16 резких склеек по 0.5 с (120 уд/мин), название в начале, музыка
cmd("project.settings", #{width: 1080, height: 1920, fps: 30});
let ids = [];
for i in 0..16 {
    let r = cmd("slot.add", #{kind: "media", at: i * 0.5, duration: 0.5, label: `Склейка ${i + 1}`});
    ids.push(r.data.id);
}
cmd("effect.add", #{ids: ids, kind: "color", params: #{contrast: 0.15, saturation: 0.1}});
cmd("effect.add", #{ids: ids, kind: "vignette", params: #{amount: 0.35}});
cmd("slot.add", #{kind: "text", at: 0, duration: 2.0, label: "Название", text: "НАЗВАНИЕ", style: #{position: "center", size: 140, "in": "pop", by: "char", anim: 0.25}});
let m = cmd("slot.add", #{kind: "music", at: 0, duration: 8, label: "Музыка (120 уд/мин)"});
cmd("effect.add", #{ids: [m.data.id], kind: "audio_fade_out", params: #{duration: 0.5}});
