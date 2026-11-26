//! name: bright_montage
//! title: Яркий промо-монтаж
//! description: стиль «промо» — для рекламы, Reels, клипов, промо, анонсов (не для фильмов и мультфильмов): делает ролик ярким поверх уже собранного монтажа: переходы Fusion из библиотеки по настроению, медленный наезд (Ken Burns) на каждый кадр, LUT по теме, свечение, блики плёнки наложением на склейках, свуши на каждой склейке и удар в начале, анимированный заголовок, нижняя треть и финальный призыв из шаблонов «Титры», музыка с затуханием
//! params: mood — настроение: epic | warm | energetic (epic); title — заголовок; subtitle — подзаголовок (нижняя треть); cta — призыв в конце; music — путь к музыке (если на таймлайне её ещё нет); lut — слова для поиска LUT (по настроению); overlays — блики на склейках (true); whoosh — свуши (true)

let mood = if params.mood == () { "epic" } else { params.mood };

// длина медиа (с), 0 — картинка/неизвестно
fn media_dur(mid) {
    for m in list(project().media) { if m.id == mid { return num(m.duration); } }
    0.0
}
// длина клипа: не длиннее нужного, остатка ролика и самого файла
fn fit_len(want, left, mid) {
    let d = min(want, left);
    let md = media_dur(mid);
    if md > 0.0 { d = min(d, md); }
    max(d, 0.1)
}
let p = project();

// основная видеодорожка — та, где больше всего клипов с медиа
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

// 1. переходы Fusion по настроению (на склейках встык)
let pool = switch mood {
    "energetic" => ["Glitch Bad Signal", "Circle Zoom 01", "Glitch HUD", "Circle Rotate and Zoom 02", "Glitch Old TV", "Circle Zoom 02"],
    "warm" => ["Light Leak 01", "Light Leak 02", "Light Leak 03", "Light Leak 04"],
    _ => ["Light Leak 01", "Circle Zoom 01", "Camera Flight", "Light Leak 03", "Circle Rotate and Zoom 02", "Light Leak 02"],
};
let cuts = [];
let i = 0;
for c in clips {
    if i > 0 {
        let prev = clips[i - 1];
        if (num(prev.start) + num(prev.duration) - num(c.start)).abs() < 0.05 {
            let tpl = pool[(i - 1) % pool.len()];
            try { cmd("transition.set", #{id: c.id, kind: "fusion", template: tpl, duration: 0.7}); }
            catch { cmd("transition.set", #{id: c.id, kind: "zoom", duration: 0.5}); }
            cuts.push(num(c.start));
        }
    }
    i += 1;
}

// 2. медленный наезд на каждый кадр (ключи масштаба), чередуя наезд и отъезд
i = 0;
for c in clips {
    let s = num(c.start); let e = s + num(c.duration);
    let a = if i % 2 == 0 { 1.0 } else { 1.12 };
    let b = if i % 2 == 0 { 1.12 } else { 1.0 };
    cmd("key.set", #{id: c.id, param: "transform.scale", at: s, value: a, ease: "ease"});
    cmd("key.set", #{id: c.id, param: "transform.scale", at: e - 0.04, value: b, ease: "ease"});
    i += 1;
}

// 3. цвет: LUT по теме + мягкое свечение
let ids = [];
for c in clips { ids.push(c.id); }
let lq = if params.lut != () { params.lut } else { switch mood { "warm" => "Свадьба", "energetic" => "Блог", _ => "Кино Cinematic" } };
try { cmd("lut.apply", #{ids: ids, name: lq, strength: 0.6}); } catch { try { cmd("lut.apply", #{ids: ids, name: "Cinematic", strength: 0.6}); } catch {} }
try { cmd("block.apply", #{ids: [clips[0].id, clips[clips.len() - 1].id], name: "Мягкое свечение", values: #{amount: 0.35}}); } catch {}

// 4. блики на склейках наложением «экран»
let lanes = 0;
if params.overlays != false && cuts.len() > 0 {
    let leaks = library("Футажи/Световые блики", if mood == "energetic" { "glitch" } else { "leak" }, 6);
    if leaks.is_empty() { leaks = library("Футажи/Световые блики", "", 6); }
    if !leaks.is_empty() {
        let tr = cmd("track.add", #{kind: "video", name: "Блики"});
        let k = 0;
        for t in cuts {
            let m = cmd("media.import", #{paths: [leaks[k % leaks.len()]]});
            let mid = list(m.data.ids)[0];
            let at = if t > 0.6 { t - 0.6 } else { 0.0 };
            let r = cmd("clip.add", #{media: mid, track: tr.data.id, at: at, duration: fit_len(1.2, total - at, mid), audio: false});
            cmd("clip.set", #{id: r.data.id, blend: "screen", volume: 0.0});
            cmd("effect.add", #{ids: [r.data.id], kind: "fade_in", params: #{duration: 0.3}});
            cmd("effect.add", #{ids: [r.data.id], kind: "fade_out", params: #{duration: 0.4}});
            k += 1; lanes += 1;
        }
    }
}

// 5. звук: музыка, свуши на склейках, удар в начале
let sfx = 0;
if params.music != () {
    let has_music = false;
    for t in list(project().tracks) { if t.kind == "audio" && t.clips != () && list(t.clips).len() > 0 && t.name == "Музыка" { has_music = true; } }
    if !has_music {
        let m = cmd("media.import", #{paths: [params.music]});
        let tr = cmd("track.add", #{kind: "audio", name: "Музыка"});
        let r = cmd("clip.add", #{media: list(m.data.ids)[0], track: tr.data.id, at: 0, duration: total});
        cmd("effect.add", #{ids: [r.data.id], kind: "audio_fade_out", params: #{duration: 1.5}});
    }
}
if params.whoosh != false {
    let ws = library("Звуки/Свуши", "", 8);
    let hits = library("Звуки/Удары и взрывы", "Bass_Drop", 2);
    let tr = cmd("track.add", #{kind: "audio", name: "Свуши"});
    let k = 0;
    for t in cuts {
        if ws.is_empty() { break; }
        let m = cmd("media.import", #{paths: [ws[k % ws.len()]]});
        let at = if t > 0.35 { t - 0.35 } else { 0.0 };
        // не длиннее 1.5 с и не за конец ролика (у некоторых свушей длинный хвост)
        let r = cmd("clip.add", #{media: list(m.data.ids)[0], track: tr.data.id, at: at, duration: fit_len(1.5, total - at, list(m.data.ids)[0])});
        cmd("clip.set", #{id: r.data.id, volume: 0.55});
        k += 1; sfx += 1;
    }
    if !hits.is_empty() {
        let m = cmd("media.import", #{paths: [hits[0]]});
        let r = cmd("clip.add", #{media: list(m.data.ids)[0], track: tr.data.id, at: 0.0, duration: fit_len(2.0, total, list(m.data.ids)[0]), mode: "overwrite"});
        cmd("clip.set", #{id: r.data.id, volume: 0.7});
        sfx += 1;
    }
}

// 6. титры из шаблонов: заголовок, нижняя треть, призыв.
// Простые надписи (без шаблона) там же по времени убираются — иначе текст двоится.
fn drop_plain_titles(a, b) {
    let ids = [];
    for t in list(project().tracks) {
        for c in list(t.clips) {
            if c.source.type != "title" { continue; }
            let tp = if c.source.style == () { () } else { c.source.style.template };
            if tp != () && tp != "" { continue; }
            let s = num(c.start); let e = s + num(c.duration);
            if s < b && e > a { ids.push(c.id); }
        }
    }
    if !ids.is_empty() { try { cmd("clip.remove", #{ids: ids}); } catch {} }
    ids.len()
}
// шаблоны, у которых текст точно встаёт (в т.ч. в вертикальном кадре):
// строка с \n — заголовок и подзаголовок шаблона
let titles = 0;
let dropped = 0;
if params.title != () {
    let tpl = switch mood { "energetic" => "Minimal_Title_06", "warm" => "Minimal_Title_02", _ => "Minimal_Title_04" };
    dropped += drop_plain_titles(0.0, 3.0);
    try { cmd("template.apply", #{name: tpl, text: params.title, at: 0.3, duration: 2.6}); titles += 1; } catch {}
}
if params.subtitle != () && total > 6.0 {
    let lt = if mood == "energetic" { "Lower_Third_02" } else { "Lower_Third_01" };
    dropped += drop_plain_titles(total * 0.4, total * 0.4 + 3.0);
    try { cmd("template.apply", #{name: lt, text: params.subtitle, at: total * 0.4, duration: 3.0}); titles += 1; } catch {}
}
if params.cta != () {
    dropped += drop_plain_titles(total - 2.6, total);
    try { cmd("template.apply", #{name: "Minimal_Title_06", text: params.cta, at: total - 2.6, duration: 2.5}); titles += 1; } catch {}
}

print(`Яркий монтаж (${mood}): переходов Fusion ${cuts.len()}, наездов ${clips.len()}, бликов ${lanes}, звуков ${sfx}, титров из шаблонов ${titles} (простых надписей убрано ${dropped})`);
