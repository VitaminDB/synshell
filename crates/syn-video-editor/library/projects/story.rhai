//! title: История в фото
//! category: slideshow
//! description: спокойное слайд-шоу 16:9 на 40 с — 10 фото с медленным наездом и наплывами, заголовок и подпись в конце
cmd("project.settings", #{width: 1920, height: 1080, fps: 30});
let ids = [];
for i in 0..10 {
    ids.push(cmd("slot.add", #{kind: "media", at: i * 4.0, duration: 4.0, label: `Фото ${i + 1}`, motion: true}).data.id);
}
for i in 1..10 { cmd("transition.set", #{id: ids[i], kind: "dissolve", duration: 1.0}); }
cmd("effect.add", #{ids: ids, kind: "vignette", params: #{amount: 0.3}});
cmd("effect.add", #{ids: [ids[0]], kind: "fade_in", params: #{duration: 1.0}});
cmd("effect.add", #{ids: [ids[9]], kind: "fade_out", params: #{duration: 1.5}});
cmd("slot.add", #{kind: "text", at: 0.8, duration: 3.2, label: "Заголовок", text: "Наша история", style: #{position: "center", size: 110, "in": "fade", by: "word", anim: 0.8}});
cmd("slot.add", #{kind: "text", at: 36.0, duration: 3.5, label: "Подпись", text: "Спасибо, что были рядом", style: #{position: "bottom", size: 60, "in": "type", by: "char"}});
let m = cmd("slot.add", #{kind: "music", at: 0, duration: 40, label: "Музыка"});
cmd("effect.add", #{ids: [m.data.id], kind: "audio_fade_in", params: #{duration: 1.5}});
cmd("effect.add", #{ids: [m.data.id], kind: "audio_fade_out", params: #{duration: 3.0}});
