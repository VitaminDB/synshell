//! name: beat_clip
//! title: Клип под музыку
//! description: трек на звуковую дорожку, ролики медиатеки нарезаются по ударам (бит — из media.index), смена кадра на каждом every-м ударе
//! params: music — трек m… (по умолчанию первый звук), clips — ролики [m…] (по умолчанию все видео), every — ударов на кадр (2), length — длина, с (весь трек, до 60)

let p = project();
let mus = params.music;
if mus == () { for m in list(p.media) { if m.kind == "audio" { mus = m.id; break; } } }
if mus == () { throw "нет трека: params.music или звук в медиатеке"; }
let every = max(1, parse_int(`${num(params.every, 2.0)}`.split(".")[0]));
let vids = if params.clips == () { [] } else { params.clips };
if vids.is_empty() { for m in list(p.media) { if m.kind == "video" { vids.push(m.id); } } }
if vids.is_empty() { throw "нет роликов для нарезки"; }

let m = ();
for x in list(p.media) { if x.id == mus { m = x; } }
if m.index == () || list(m.index.beats).is_empty() {
    cmd("media.index", #{ids: [mus], captions: false, speakers: false});
    for x in list(project().media) { if x.id == mus { m = x; } }
}
if list(m.index.beats).is_empty() { throw "в треке не найдены ни бит, ни акценты (не музыка?)"; }
let len = min(num(params.length, num(m.duration)), num(m.duration));
len = min(len, 60.0);
cmd("clip.add", #{media: mus, at: 0, duration: len});

// длительности роликов
let durs = [];
for v in vids { for x in list(p.media) { if x.id == v { durs.push(num(x.duration)); } } }
let offs = [];
for v in vids { offs.push(0.0); }

let beats = [];
for b in list(m.index.beats) { if num(b) < len { beats.push(num(b)); } }
let cuts = [0.0];
let i = every;
// склейки не ближе 0.3 с друг к другу и к концу — без коротких обрывков и зазоров
while i < beats.len() { if beats[i] - cuts[cuts.len() - 1] >= 0.3 && len - beats[i] >= 0.3 { cuts.push(beats[i]); } i += every; }
cuts.push(len);
let k = 0;
let n = 0;
for j in 0..cuts.len() - 1 {
    let a = cuts[j]; let b = cuts[j + 1];
    let vi = k % vids.len();
    let d = b - a;
    if offs[vi] + d > durs[vi] { offs[vi] = 0.0; }
    let r = cmd("clip.add", #{media: vids[vi], at: a, offset: offs[vi], duration: d, audio: false});
    // звук роликов не нужен — играет трек
    if r.data.id != () { cmd("clip.set", #{id: r.data.id, volume: 0.0}); }
    offs[vi] += d;
    k += 1; n += 1;
}
let tempo = if m.index.bpm == () { "ровного темпа нет — по сильным акцентам" } else { `темп ${m.index.bpm} уд/мин` };
print(`${tempo}, кадров ${n} по ${every} удара, длина ${len} с`);
