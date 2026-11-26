//! title: Reels-слайд-шоу
//! category: reels
//! description: вертикальный ролик 15 с — 6 видео/фото с наездом и переходами, заголовок вначале, призыв в конце, музыка
// Шаблон проекта: строит проект командами; места — slot.add.
cmd("project.settings", #{width: 1080, height: 1920, fps: 30});
let kinds = ["zoom", "slide_left", "slide_up", "zoom", "slide_right"];
let ids = [];
for i in 0..6 {
    let r = cmd("slot.add", #{kind: "media", at: i * 2.5, duration: 2.5, label: `Кадр ${i + 1}`, motion: true});
    ids.push(r.data.id);
}
for i in 1..6 { cmd("transition.set", #{id: ids[i], kind: kinds[i - 1], duration: 0.4}); }
cmd("effect.add", #{ids: ids, kind: "color", params: #{contrast: 0.08, saturation: 0.12}});
cmd("slot.add", #{kind: "text", at: 0.2, duration: 2.2, label: "Заголовок", text: "ЗАГОЛОВОК", style: #{position: "center", size: 120, by: "char", "in": "pop", out: "fade", box: false}});
cmd("slot.add", #{kind: "text", at: 12.7, duration: 2.3, label: "Призыв", text: "Подписывайтесь!", style: #{position: "bottom", size: 80, box: true, "in": "slide", by: "word"}});
let m = cmd("slot.add", #{kind: "music", at: 0, duration: 15, label: "Музыка"});
cmd("effect.add", #{ids: [m.data.id], kind: "audio_fade_out", params: #{duration: 1.5}});
