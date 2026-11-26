//! name: calm_montage
//! title: Спокойный монтаж
//! description: стиль «спокойный / инфо» — для инструкций, интервью, обзоров, презентаций, по умолчанию, если жанр неясен: короткие наплывы на склейках, лёгкая цветокоррекция, нижние трети с подписями, ровная тихая музыка под голос, без бликов, свушей и резких эффектов
//! params: title — заголовок (нижняя треть в начале); notes — список подписей по ходу ролика; music — путь к музыке; volume — громкость музыки (0.35)

let p = project();
let main = ();
let best = 0;
for t in list(p.tracks) {
    if t.kind != "video" { continue; }
    let n = 0;
    for c in list(t.clips) { if c.source.media != () { n += 1; } }
    if n > best { best = n; main = t; }
}
if main == () { throw "на таймлайне нет видеоклипов — сначала соберите монтаж"; }
let clips = [];
for c in list(main.clips) { if c.source.media != () { clips.push(c); } }
let total = 0.0;
for c in clips { let e = num(c.start) + num(c.duration); if e > total { total = e; } }

let i = 0; let tr_n = 0;
for c in clips {
    if i > 0 {
        let prev = clips[i - 1];
        if (num(prev.start) + num(prev.duration) - num(c.start)).abs() < 0.05 {
            cmd("transition.set", #{id: c.id, kind: "dissolve", duration: 0.4});
            tr_n += 1;
        }
    }
    i += 1;
}
let ids = [];
for c in clips { ids.push(c.id); }
cmd("effect.add", #{ids: ids, kind: "color", params: #{contrast: 0.05, saturation: 0.05}});

let titles = 0;
if params.title != () {
    try { cmd("template.apply", #{name: "Lower_Third_02", text: params.title, at: 0.5, duration: 3.5}); titles += 1; } catch {
        cmd("title.add", #{text: params.title, at: 0.5, duration: 3.5, style: #{position: "bottom", box: true}}); titles += 1;
    }
}
let notes = list(params.notes);
if notes.len() > 0 {
    let step = total / (notes.len() + 1);
    let k = 1;
    for n in notes {
        cmd("title.add", #{text: n, at: step * k, duration: min(3.0, step * 0.9), style: #{position: "bottom", box: true, "in": "fade", out: "fade"}});
        k += 1; titles += 1;
    }
}
if params.music != () {
    let m = cmd("media.import", #{paths: [params.music]});
    let tr = cmd("track.add", #{kind: "audio", name: "Музыка"});
    let r = cmd("clip.add", #{media: list(m.data.ids)[0], track: tr.data.id, at: 0, duration: total});
    cmd("clip.set", #{id: r.data.id, volume: num(params.volume, 0.35)});
    cmd("effect.add", #{ids: [r.data.id], kind: "audio_fade_out", params: #{duration: 2.0}});
}
print(`Спокойный монтаж: наплывов ${tr_n}, подписей ${titles}`);
