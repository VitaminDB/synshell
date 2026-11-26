//! name: talking_head
//! title: Говорящая голова
//! description: паузы вырезаны (джамп-каты), наезд на каждом втором куске прячет склейку, субтитры
//! params: media — медиа m… (по умолчанию первое видео, если таймлайн пуст), zoom — наезд (1.15), subtitles — субтитры (true), style — оформление субтитров (boxed)

fn video_clips(p) {
    let out = [];
    for t in list(p.tracks) { if t.kind == "video" && t.name != "Субтитры" { for c in list(t.clips) { if c.source.media != () { out.push(c); } } } }
    out
}

let p = project();
let zoom = num(params.zoom, 1.15);
let want_subs = if params.subtitles == () { true } else { params.subtitles };
let style = if params.style == () { "boxed" } else { params.style };
if video_clips(p).is_empty() {
    let mid = params.media;
    if mid == () { for m in list(p.media) { if m.kind == "video" { mid = m.id; break; } } }
    if mid == () { throw "в медиатеке нет видео"; }
    cmd("clip.add", #{media: mid, at: 0});
    p = project();
}
let cut = 0.0;
for c in video_clips(p) {
    let r = cmd("clip.cut_silence", #{id: c.id});
    cut += num(r.data.removed);
}
// наезд на каждом втором куске (по порядку на таймлайне)
p = project();
let cs = video_clips(p);
cs.sort(|a, b| if num(a.start) < num(b.start) { -1 } else { 1 });
let n = 0;
for i in 0..cs.len() {
    if i % 2 == 1 {
        let c = cs[i];
        let has = false;
        for e in list(c.effects) { if e.kind == "transform" { has = true; cmd("effect.set", #{id: e.id, params: #{scale: zoom}}); } }
        if !has { cmd("effect.add", #{ids: [c.id], kind: "transform", params: #{scale: zoom}}); }
        n += 1;
    }
}
let subs = "без субтитров";
if want_subs {
    for c in video_clips(project()) {
        for m in list(project().media) { if m.id == c.source.media && m.has_audio && m.transcript == () { cmd("media.transcribe", #{id: m.id}); } }
    }
    subs = cmd("subtitles.add", #{style: style}).text;
}
print(`вырезано пауз ${(cut * 10.0).round() / 10.0} с, кусков ${cs.len()}, наездов ${n}; ${subs}`);
