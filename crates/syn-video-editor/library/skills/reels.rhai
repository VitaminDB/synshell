//! name: reels
//! title: Reels / Shorts
//! description: вертикальный ролик 9:16 — кадр заполнен (обрезка по бокам), паузы в речи вырезаны, длина не больше length, крупные субтитры по словам
//! params: media — медиа m… (по умолчанию первое видео; если на таймлайне уже есть клипы — работает с ними), length — длина, с (60), subtitles — субтитры (true), style — оформление субтитров (reels)

fn video_clips(p) {
    let out = [];
    for t in list(p.tracks) { if t.kind == "video" && t.name != "Субтитры" { for c in list(t.clips) { if c.source.media != () { out.push(c); } } } }
    out
}

let p = project();
let len = num(params.length, 60.0);
let want_subs = if params.subtitles == () { true } else { params.subtitles };
let style = if params.style == () { "reels" } else { params.style };

// что монтируем
if video_clips(p).is_empty() {
    let mid = params.media;
    if mid == () {
        for m in list(p.media) { if m.kind == "video" { mid = m.id; break; } }
    }
    if mid == () { throw "в медиатеке нет видео"; }
    cmd("clip.add", #{media: mid, at: 0});
}

// кадр 9:16
cmd("project.settings", #{width: 1080, height: 1920});

// паузы в речи
p = project();
let cut = 0.0;
for c in video_clips(p) {
    let m = ();
    for x in list(p.media) { if x.id == c.source.media { m = x; } }
    if m.has_audio {
        let r = cmd("clip.cut_silence", #{id: c.id});
        cut += num(r.data.removed);
    }
}

// заполнить кадр: из «вписать» в «заполнить»
p = project();
for c in video_clips(p) {
    let m = ();
    for x in list(p.media) { if x.id == c.source.media { m = x; } }
    if m.width > 0 && m.height > 0 {
        let w = num(m.width); let h = num(m.height);
        let contain = min(1080.0 / w, 1920.0 / h);
        let cover = max(1080.0 / w, 1920.0 / h);
        let k = cover / contain;
        if k > 1.001 {
            let has = false;
            for e in list(c.effects) { if e.kind == "transform" { has = true; cmd("effect.set", #{id: e.id, params: #{scale: k}}); } }
            if !has { cmd("effect.add", #{ids: [c.id], kind: "transform", params: #{scale: k}}); }
        }
    }
}

// длина
p = project();
let removed = 0;
for t in list(p.tracks) {
    for c in list(t.clips) {
        let end = num(c.start) + num(c.duration);
        if num(c.start) >= len { cmd("clip.remove", #{ids: [c.id]}); removed += 1; }
        else if end > len { cmd("clip.trim", #{id: c.id, end: len}); }
    }
}

// субтитры
let subs = "без субтитров";
if want_subs {
    p = project();
    for c in video_clips(p) {
        for m in list(p.media) { if m.id == c.source.media && m.has_audio && m.transcript == () { cmd("media.transcribe", #{id: m.id}); } }
    }
    let r = cmd("subtitles.add", #{style: style});
    subs = r.text;
}
print(`9:16, вырезано пауз ${(cut * 10.0).round() / 10.0} с, за пределами ${len} с убрано клипов: ${removed}; ${subs}`);
