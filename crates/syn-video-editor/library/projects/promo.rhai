//! title: Промо
//! category: promo
//! description: рекламный ролик 16:9 на 15 с — 4 кадра со сдвигами и подписями-преимуществами, финальная карточка с названием и призывом
cmd("project.settings", #{width: 1920, height: 1080, fps: 30});
let ids = [];
for i in 0..4 {
    ids.push(cmd("slot.add", #{kind: "media", at: i * 3.0, duration: 3.0, label: `Кадр ${i + 1}`, motion: true}).data.id);
}
for i in 1..4 { cmd("transition.set", #{id: ids[i], kind: if i % 2 == 1 { "slide_left" } else { "wipe_left" }, duration: 0.5}); }
cmd("effect.add", #{ids: ids, kind: "color", params: #{contrast: 0.1, saturation: 0.1, temperature: 0.05}});
let labels = ["Преимущество 1", "Преимущество 2", "Преимущество 3"];
for i in 0..3 {
    cmd("slot.add", #{kind: "text", at: 3.0 * (i + 1) + 0.3, duration: 2.4, label: labels[i], text: labels[i], style: #{position: "bottom", size: 64, box: true, "in": "slide", by: "word"}});
}
cmd("slot.add", #{kind: "text", at: 0.3, duration: 2.5, label: "Название", text: "НАЗВАНИЕ", style: #{position: "center", size: 130, "in": "pop", by: "char"}});
cmd("slot.add", #{kind: "media", at: 12, duration: 3, label: "Фон финала", track: "Кадры"});
cmd("slot.add", #{kind: "text", at: 12.2, duration: 2.8, label: "Призыв", text: "Закажите сейчас", style: #{position: "center", size: 96, "in": "pop", by: "word"}, track: "Финал"});
cmd("slot.add", #{kind: "text", at: 12.6, duration: 2.4, label: "Сайт", text: "example.com", style: #{position: "bottom", size: 52, color: "#ffffffcc"}, track: "Финал 2"});
let m = cmd("slot.add", #{kind: "music", at: 0, duration: 15, label: "Музыка"});
cmd("effect.add", #{ids: [m.data.id], kind: "audio_fade_out", params: #{duration: 1.5}});
