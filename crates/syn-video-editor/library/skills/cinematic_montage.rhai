//! name: cinematic_montage
//! title: Кинематографичный монтаж
//! description: стиль «кино» — для фильмов, мультфильмов, историй, сцен с сюжетом: прямые склейки по действию, редкие наплывы и затемнения между частями, проявление из чёрного и затемнение в конце, кинокаше 2.39:1, мягкий киношный LUT и лёгкое плёночное зерно, без свушей, глитчей и прыгающих титров; спокойный заголовок и финальная карточка, музыка по настроению тихо с нарастанием и затуханием
//! params: title — название (на чёрном/первом кадре); credits — финальная надпись; music — путь к музыке (если её нет на таймлайне); every — наплыв на каждой N-й склейке (3); letterbox — кинокаше (true); grain — зерно плёнки (true)

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
let every = num(params.every, 3.0);

// 1. склейки: прямые; каждая N-я — мягкий наплыв (смена части)
let i = 0; let soft = 0;
for c in clips {
    if i > 0 {
        let prev = clips[i - 1];
        let adj = (num(prev.start) + num(prev.duration) - num(c.start)).abs() < 0.05;
        if adj && i % every.to_int() == 0 {
            cmd("transition.set", #{id: c.id, kind: "dissolve", duration: 0.9});
            soft += 1;
        }
    }
    i += 1;
}
// проявление из чёрного и затемнение в конце
cmd("effect.add", #{ids: [clips[0].id], kind: "fade_in", params: #{duration: 1.2}});
cmd("effect.add", #{ids: [clips[clips.len() - 1].id], kind: "fade_out", params: #{duration: 1.5}});

// 2. цвет: мягкий киношный LUT
let ids = [];
for c in clips { ids.push(c.id); }
try { cmd("lut.apply", #{ids: ids, name: "Кино Cinematic", strength: 0.4}); } catch { try { cmd("lut.apply", #{ids: ids, name: "Cinematic", strength: 0.4}); } catch {} }

// 3. кинокаше 2.39:1 — чёрные полосы сверху и снизу
let s = p.settings;
let bars = 0;
if params.letterbox != false && num(s.width) > num(s.height) {
    let bh = ((num(s.height) - num(s.width) / 2.39) / 2.0) / num(s.height);
    if bh > 0.01 {
        let tr = cmd("track.add", #{kind: "video", name: "Каше"});
        for y in [-0.5 + bh / 2.0, 0.5 - bh / 2.0] {
            let r = cmd("shape.add", #{kind: "rect", at: 0, duration: total, track: tr.data.id, style: #{fill: "#000000", width: 1.02, height: bh, stroke_width: 0}});
            cmd("effect.add", #{ids: [r.data.id], kind: "transform", params: #{y: y}});
            bars += 1;
        }
    }
}

// 4. лёгкое плёночное зерно наложением на всю длину
let grain = 0;
if params.grain != false {
    let g = library("Футажи/Плёнка, шум и царапины", "grain", 1);
    if g.is_empty() { g = library("Футажи/Плёнка, шум и царапины", "35MM", 1); }
    if !g.is_empty() {
        let m = cmd("media.import", #{paths: [g[0]]});
        let tr = cmd("track.add", #{kind: "video", name: "Плёнка"});
        let gdur = 10.0;
        for mm in list(project().media) { if mm.id == list(m.data.ids)[0] && num(mm.duration) > 0.0 { gdur = num(mm.duration); } }
        let at = 0.0;
        while at < total - 0.05 {
            let r = cmd("clip.add", #{media: list(m.data.ids)[0], track: tr.data.id, at: at, duration: min(min(10.0, total - at), max(0.5, gdur)), audio: false});
            cmd("clip.set", #{id: r.data.id, blend: "overlay", volume: 0.0});
            cmd("effect.add", #{ids: [r.data.id], kind: "transform", params: #{opacity: 0.25}});
            at += min(10.0, max(0.5, gdur)); grain += 1;
        }
    }
}

// 5. музыка тихо, с нарастанием и затуханием
if params.music != () {
    let m = cmd("media.import", #{paths: [params.music]});
    let tr = cmd("track.add", #{kind: "audio", name: "Музыка"});
    let r = cmd("clip.add", #{media: list(m.data.ids)[0], track: tr.data.id, at: 0, duration: total});
    cmd("clip.set", #{id: r.data.id, volume: 0.6});
    cmd("effect.add", #{ids: [r.data.id], kind: "audio_fade_in", params: #{duration: 2.0}});
    cmd("effect.add", #{ids: [r.data.id], kind: "audio_fade_out", params: #{duration: 3.0}});
}

// 6. спокойные надписи: название и финальная карточка (проявление без прыжков)
let titles = 0;
if params.title != () {
    cmd("title.add", #{text: params.title, at: 0.6, duration: 3.0, style: #{position: "center", size: num(s.height) * 0.07, "in": "fade", out: "fade", anim: 0.8, font: "Playfair Display"}});
    titles += 1;
}
if params.credits != () {
    cmd("title.add", #{text: params.credits, at: max(0.0, total - 3.5), duration: 3.3, style: #{position: "center", size: num(s.height) * 0.05, "in": "fade", out: "fade", anim: 1.0}});
    titles += 1;
}
print(`Кинематографичный монтаж: прямых склеек ${clips.len() - 1 - soft}, наплывов ${soft}, каше ${bars > 0}, зерно ${grain > 0}, надписей ${titles}`);
