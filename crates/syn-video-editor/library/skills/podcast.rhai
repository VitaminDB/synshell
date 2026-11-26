//! name: podcast
//! title: Подкаст
//! description: длинные паузы вырезаны, при смене диктора — подпись «Диктор N» вверху кадра (дикторы — из media.index), по желанию субтитры
//! params: media — медиа m… (по умолчанию первое видео или звук, если таймлайн пуст), names — имена дикторов [«Анна», «Борис»] (по порядку), min_pause — минимальная пауза, с (0.8), subtitles — субтитры (false)

fn media_clips(p) {
    let out = [];
    for t in list(p.tracks) { if t.name != "Субтитры" { for c in list(t.clips) { if c.source.media != () && c.link == () || c.source.media != () && t.kind == "video" { out.push(c); } } } }
    out
}

let p = project();
let min_pause = num(params.min_pause, 0.8);
if media_clips(p).is_empty() {
    let mid = params.media;
    if mid == () { for m in list(p.media) { if m.kind == "video" || m.kind == "audio" { mid = m.id; break; } } }
    if mid == () { throw "в медиатеке нет видео или звука"; }
    cmd("clip.add", #{media: mid, at: 0});
    p = project();
}
// дикторы
let mids = [];
for c in media_clips(p) { if !mids.contains(c.source.media) { mids.push(c.source.media); } }
for mid in mids {
    let m = ();
    for x in list(project().media) { if x.id == mid { m = x; } }
    if m.index == () || list(m.index.speakers).is_empty() { cmd("media.index", #{ids: [mid], captions: false}); }
}
// паузы
let cut = 0.0;
for c in media_clips(project()) {
    let r = cmd("clip.cut_silence", #{id: c.id, min: min_pause});
    cut += num(r.data.removed);
}
// подписи при смене диктора: время источника → таймлайн по кускам
p = project();
let names = if params.names == () { [] } else { params.names };
let shown = 0;
let last = "";
let cs = media_clips(p);
cs.sort(|a, b| if num(a.start) < num(b.start) { -1 } else { 1 });
for c in cs {
    let m = ();
    for x in list(p.media) { if x.id == c.source.media { m = x; } }
    if m.index == () || list(m.index.speakers).is_empty() { continue; }
    let off = num(c.offset); let sp = num(c.speed, 1.0); let dur = num(c.duration);
    for s in list(m.index.speakers) {
        let st = num(s.start);
        if st < off || st >= off + dur * sp { continue; }
        if s.speaker == last { continue; }
        last = s.speaker;
        let i = parse_int(s.speaker) - 1;
        let label = if i >= 0 && i < names.len() { names[i] } else { `Диктор ${s.speaker}` };
        let at = num(c.start) + (st - off) / sp;
        cmd("title.add", #{text: label, at: at, duration: 2.5, style: #{position: "top", size: 40, box: true, "in": "fade", "out": "fade"}});
        shown += 1;
    }
}
let subs = "";
if params.subtitles == true {
    for mid in mids { for m in list(project().media) { if m.id == mid && m.transcript == () { cmd("media.transcribe", #{id: mid}); } } }
    subs = "; " + cmd("subtitles.add", #{style: "boxed"}).text;
}
print(`вырезано пауз ${(cut * 10.0).round() / 10.0} с, подписей дикторов ${shown}${subs}`);
