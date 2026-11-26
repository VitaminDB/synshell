//! title: Путешествие
//! category: slideshow
//! description: тёплый ролик 16:9 на 22 с — заголовок места поверх первого кадра, 8 кадров с наплывами, подпись в конце
cmd("project.settings", #{width: 1920, height: 1080, fps: 30});
let ids = [];
for i in 0..8 {
    ids.push(cmd("slot.add", #{kind: "media", at: i * 2.75, duration: 2.75, label: `Кадр ${i + 1}`, motion: true}).data.id);
}
for i in 1..8 { cmd("transition.set", #{id: ids[i], kind: "dissolve", duration: 0.7}); }
cmd("effect.add", #{ids: ids, kind: "grade", params: #{gain_x: 0.87, gain_y: 0.5, lift_x: -1.0, lift_y: 0.0, gamma: 0.03}});
cmd("effect.add", #{ids: [ids[0]], kind: "fade_in", params: #{duration: 0.8}});
cmd("effect.add", #{ids: [ids[7]], kind: "fade_out", params: #{duration: 1.0}});
cmd("slot.add", #{kind: "text", at: 0.6, duration: 3.0, label: "Место", text: "ИСЛАНДИЯ", style: #{position: "center", size: 150, by: "char", "in": "scatter", anim: 0.8}});
cmd("slot.add", #{kind: "text", at: 1.4, duration: 2.2, label: "Дата", text: "лето 2026", style: #{position: "bottom", size: 52, "in": "fade", color: "#ffffffcc"}, track: "Даты"});
cmd("slot.add", #{kind: "text", at: 19.0, duration: 3.0, label: "Финал", text: "До встречи!", style: #{position: "center", size: 96, "in": "type", by: "char"}});
let m = cmd("slot.add", #{kind: "music", at: 0, duration: 22, label: "Музыка"});
cmd("effect.add", #{ids: [m.data.id], kind: "audio_fade_in", params: #{duration: 1.0}});
cmd("effect.add", #{ids: [m.data.id], kind: "audio_fade_out", params: #{duration: 2.0}});
