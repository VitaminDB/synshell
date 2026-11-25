use serde_json::json;

use super::*;
use crate::doc::{Media, MediaKind};

fn ed_with_media(dur: f64) -> Editor {
    let mut ed = Editor::new();
    let id = ed.project.alloc();
    ed.project.media.push(Media { id, path: "/x/a.mp4".into(), kind: MediaKind::Video, duration: dur, width: 1920, height: 1080, fps: 30.0, has_audio: true, transcript: None });
    ed
}

fn clips(ed: &Editor, ti: usize) -> Vec<(f64, f64, f64)> {
    ed.project.tracks[ti].clips.iter().map(|c| (c.start, c.duration, c.offset)).collect()
}

#[test]
fn every_command_has_schema_and_runs_headless() {
    for c in all() {
        let s = c.schema();
        assert_eq!(s["type"], "object", "{}", c.id);
        assert!(!c.help.is_empty() && !c.title.is_empty(), "{}", c.id);
        for a in c.args {
            assert!(s["properties"][a.name]["description"].is_string(), "{}.{}", c.id, a.name);
        }
    }
    let mut ids: Vec<_> = all().iter().map(|c| c.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), all().len(), "повтор id");
}

#[test]
fn add_split_trim_remove_undo() {
    let mut ed = ed_with_media(10.0);
    run(&mut ed, "clip.add", &json!({"media": "m3"})).unwrap();
    run(&mut ed, "clip.add", &json!({"media": 3, "offset": 2, "duration": 4})).unwrap();
    assert_eq!(clips(&ed, 0), vec![(0.0, 10.0, 0.0), (10.0, 4.0, 2.0)]);
    // разрез первого клипа в 2.5 с
    run(&mut ed, "clip.split", &json!({"at": "0:02.5"})).unwrap();
    assert_eq!(clips(&ed, 0), vec![(0.0, 2.5, 0.0), (2.5, 7.5, 2.5), (10.0, 4.0, 2.0)]);
    let right = ed.project.tracks[0].clips[1].id;
    // удалить середину со сдвигом
    run(&mut ed, "clip.remove", &json!({"ids": [right], "ripple": true})).unwrap();
    assert_eq!(clips(&ed, 0), vec![(0.0, 2.5, 0.0), (2.5, 4.0, 2.0)]);
    // обрезка начала второго: offset растёт
    let last = ed.project.tracks[0].clips[1].id;
    run(&mut ed, "clip.trim", &json!({"id": last, "start": 3.0})).unwrap();
    assert_eq!(clips(&ed, 0)[1], (3.0, 3.5, 2.5));
    // нельзя вытянуть за границы источника
    run(&mut ed, "clip.trim", &json!({"id": last, "end": 100})).unwrap();
    assert_eq!(clips(&ed, 0)[1], (3.0, 7.5, 2.5));
    for _ in 0..6 {
        run(&mut ed, "edit.undo", &json!({})).unwrap();
    }
    assert!(ed.project.tracks[0].clips.is_empty());
    run(&mut ed, "edit.redo", &json!({})).unwrap();
    assert_eq!(clips(&ed, 0), vec![(0.0, 10.0, 0.0)]);
}

#[test]
fn overwrite_and_insert() {
    let mut ed = ed_with_media(10.0);
    run(&mut ed, "clip.add", &json!({"media": 3})).unwrap();
    // перезапись середины: клип режется на два
    run(&mut ed, "clip.add", &json!({"media": 3, "at": 4, "duration": 2})).unwrap();
    assert_eq!(clips(&ed, 0), vec![(0.0, 4.0, 0.0), (4.0, 2.0, 0.0), (6.0, 4.0, 6.0)]);
    // вставка: всё правее сдвигается
    run(&mut ed, "clip.add", &json!({"media": 3, "at": 1, "duration": 1, "mode": "insert"})).unwrap();
    assert_eq!(clips(&ed, 0), vec![(0.0, 1.0, 0.0), (1.0, 1.0, 0.0), (2.0, 3.0, 1.0), (5.0, 2.0, 0.0), (7.0, 4.0, 6.0)]);
}

#[test]
fn agent_group_is_one_undo_step() {
    let mut ed = ed_with_media(10.0);
    ed.begin_group("ИИ: монтаж");
    run(&mut ed, "clip.add", &json!({"media": 3})).unwrap();
    run(&mut ed, "title.add", &json!({"text": "Привет", "at": 1})).unwrap();
    run(&mut ed, "clip.split", &json!({"at": 5})).unwrap();
    ed.end_group();
    assert_eq!(ed.undo_label(), Some("ИИ: монтаж"));
    run(&mut ed, "edit.undo", &json!({})).unwrap();
    assert!(ed.project.tracks.iter().all(|t| t.clips.is_empty()));
    assert!(run(&mut ed, "edit.undo", &json!({})).is_err());
}

#[test]
fn errors_are_explained() {
    let mut ed = ed_with_media(10.0);
    let e = run(&mut ed, "clip.add", &json!({})).unwrap_err();
    assert!(e.contains("media"), "{e}");
    let e = run(&mut ed, "clip.add", &json!({"media": 3, "bogus": 1})).unwrap_err();
    assert!(e.contains("bogus"), "{e}");
    let e = run(&mut ed, "effect.add", &json!({"ids": [1], "kind": "telepor"})).unwrap_err();
    assert!(e.contains("нет эффекта"), "{e}");
    assert!(find("clip_split").is_some());
}

#[test]
fn speed_changes_length() {
    let mut ed = ed_with_media(10.0);
    run(&mut ed, "clip.add", &json!({"media": 3})).unwrap();
    let id = ed.project.tracks[0].clips[0].id;
    run(&mut ed, "clip.set", &json!({"id": id, "speed": 2})).unwrap();
    assert_eq!(clips(&ed, 0), vec![(0.0, 5.0, 0.0)]);
}

#[test]
fn cut_source_ripples() {
    let mut ed = ed_with_media(10.0);
    run(&mut ed, "clip.add", &json!({"media": 3})).unwrap();
    run(&mut ed, "clip.add", &json!({"media": 3, "duration": 2})).unwrap();
    let id = ed.project.tracks[0].clips[0].id;
    // вырезать 2–3 и 6–8 с источника: остаётся 0–2, 3–6, 8–10 (7 с), второй клип сдвигается на 3 с
    run(&mut ed, "clip.cut_ranges", &json!({"id": id, "ranges": [[6, 8], [2, 3]]})).unwrap();
    assert_eq!(clips(&ed, 0), vec![(0.0, 2.0, 0.0), (2.0, 3.0, 3.0), (5.0, 2.0, 8.0), (7.0, 2.0, 0.0)]);
}

#[test]
fn selection_shift_copy_paste() {
    let mut ed = ed_with_media(10.0);
    run(&mut ed, "clip.add", &json!({"media": 3, "duration": 2})).unwrap();
    run(&mut ed, "clip.add", &json!({"media": 3, "duration": 2})).unwrap();
    let (a, b) = (ed.project.tracks[0].clips[0].id, ed.project.tracks[0].clips[1].id);
    // разрез и удаление без ids — по выделению
    run(&mut ed, "selection.set", &json!({"ids": [b]})).unwrap();
    run(&mut ed, "transition.set", &json!({"id": b, "kind": "dissolve", "duration": 0.5})).unwrap();
    run(&mut ed, "clip.split", &json!({"at": 3})).unwrap();
    let parts = &ed.project.tracks[0].clips;
    assert_eq!(parts.len(), 3);
    assert!(parts[1].transition.is_some() && parts[2].transition.is_none(), "переход остаётся у левой части");
    // сдвиг двух клипов вправо и обратно; влево за ноль — ошибка
    run(&mut ed, "selection.set", &json!({"ids": [a, b]})).unwrap();
    run(&mut ed, "clip.shift", &json!({"by": -0.5})).unwrap_err();
    run(&mut ed, "clip.shift", &json!({"ids": [b], "by": -0.5})).unwrap_err();
    let last = ed.project.tracks[0].clips[2].id;
    run(&mut ed, "clip.shift", &json!({"ids": [last], "by": 1})).unwrap();
    run(&mut ed, "clip.shift", &json!({"ids": [last], "by": -0.5})).unwrap();
    assert_eq!(ed.project.clip(last).unwrap().start, 3.5);
    // копия двух связанных видеоклипов в 10 с: едут вместе со звуком, взаимное расположение и связи сохранены
    run(&mut ed, "clip.copy", &json!({})).unwrap();
    run(&mut ed, "clip.paste", &json!({"at": 10})).unwrap();
    assert_eq!(ed.selection.len(), 4);
    let v_pasted: Vec<u64> = ed.selection.iter().copied().filter(|id| ed.project.find_clip(*id).map(|f| f.0) == Some(0)).collect();
    let a_pasted: Vec<u64> = ed.selection.iter().copied().filter(|id| ed.project.find_clip(*id).map(|f| f.0) == Some(1)).collect();
    assert_eq!(v_pasted.len(), 2);
    assert_eq!(a_pasted.len(), 2);
    let starts: Vec<f64> = v_pasted.iter().map(|id| ed.project.clip(*id).unwrap().start).collect();
    assert_eq!(starts, vec![10.0, 12.0]);
    assert_eq!(ed.project.linked_peer(v_pasted[0]), Some(a_pasted[0]));
    let n = ed.project.tracks.len();
    // вставка поверх занятого — на новые видео- и аудиодорожку
    run(&mut ed, "clip.paste", &json!({"at": 0})).unwrap();
    assert_eq!(ed.project.tracks.len(), n + 2);
    // удалить выделенное
    run(&mut ed, "clip.remove", &json!({})).unwrap();
    assert!(ed.project.tracks[n].clips.is_empty());
    assert!(ed.project.tracks[n + 1].clips.is_empty());
}

#[test]
fn detach_audio() {
    let mut ed = ed_with_media(10.0);
    // по умолчанию видео со звуком сразу разбивается на V1 + связанный звук на A1
    let added = run(&mut ed, "clip.add", &json!({"media": 3, "offset": 1, "duration": 4})).unwrap();
    let v = added.data["id"].as_u64().unwrap();
    let a = added.data["audio_id"].as_u64().unwrap();
    assert_eq!(ed.project.clip(v).unwrap().volume, 0.0);
    assert_eq!(ed.project.clip(a).unwrap().volume, 1.0);
    assert_eq!(ed.project.linked_peer(v), Some(a));
    assert_eq!(ed.project.linked_peer(a), Some(v));
    assert_eq!(clips(&ed, 0), vec![(0.0, 4.0, 1.0)]);
    assert_eq!(clips(&ed, 1), vec![(0.0, 4.0, 1.0)]);
    // звук не двоится: в audio_graph ровно один вход (с A1)
    let (inputs, _) = crate::render::export::audio_graph(&ed.project, 0.0, 4.0, 0).unwrap();
    assert_eq!(inputs.iter().filter(|s| *s == "-i").count(), 1);
    // синхронные правки: сдвиг, обрезка, скорость, громкость, эффекты, разрез
    run(&mut ed, "clip.move", &json!({"id": v, "at": 2})).unwrap();
    assert_eq!(ed.project.clip(a).unwrap().start, 2.0);
    run(&mut ed, "clip.trim", &json!({"id": v, "start": 3})).unwrap();
    assert_eq!((ed.project.clip(a).unwrap().start, ed.project.clip(a).unwrap().duration, ed.project.clip(a).unwrap().offset), (3.0, 3.0, 2.0));
    run(&mut ed, "clip.set", &json!({"id": v, "volume": 0.5, "speed": 1.5})).unwrap();
    assert_eq!(ed.project.clip(v).unwrap().volume, 0.0);
    assert_eq!(ed.project.clip(a).unwrap().volume, 0.5);
    assert_eq!(ed.project.clip(a).unwrap().speed, 1.5);
    assert_eq!(ed.project.clip(a).unwrap().duration, ed.project.clip(v).unwrap().duration);
    run(&mut ed, "effect.add", &json!({"ids": [v, a], "kind": "audio_fade_in"})).unwrap();
    assert!(ed.project.clip(v).unwrap().effects.is_empty());
    assert_eq!(ed.project.clip(a).unwrap().effects.len(), 1);
    run(&mut ed, "effect.add", &json!({"ids": [v, a], "kind": "blur"})).unwrap();
    assert_eq!(ed.project.clip(v).unwrap().effects.len(), 1);
    assert_eq!(ed.project.clip(a).unwrap().effects.len(), 1);
    run(&mut ed, "clip.split", &json!({"at": 4, "ids": [v]})).unwrap();
    assert_eq!(ed.project.tracks[0].clips.len(), 2);
    assert_eq!(ed.project.tracks[1].clips.len(), 2);
    let v2 = ed.project.tracks[0].clips[1].id;
    let a2 = ed.project.tracks[1].clips[1].id;
    assert_eq!(ed.project.linked_peer(v2), Some(a2));
    // отвязать звук: разрывает связь, дальше двигаются независимо
    // несколько клипов сразу: одним шагом отмены, у пары громкость — у звука
    let undo_before = ed.undo_len();
    run(&mut ed, "clip.set", &json!({"ids": [v, v2, a, a2], "speed": 2, "volume": 0.25})).unwrap();
    assert_eq!(ed.undo_len(), undo_before + 1);
    for id in [v, v2, a, a2] {
        assert_eq!(ed.project.clip(id).unwrap().speed, 2.0);
    }
    assert_eq!((ed.project.clip(v2).unwrap().volume, ed.project.clip(a2).unwrap().volume), (0.0, 0.25));
    // ошибка у любого — не меняется ни один
    let snap = serde_json::to_string(&ed.project).unwrap();
    run(&mut ed, "clip.set", &json!({"ids": [v, 999999], "speed": 1})).unwrap_err();
    run(&mut ed, "clip.set", &json!({"ids": [v], "id": v, "speed": 1})).unwrap_err();
    assert_eq!(serde_json::to_string(&ed.project).unwrap(), snap);
    assert_eq!(ed.undo_len(), undo_before + 1);
    run(&mut ed, "edit.undo", &json!({})).unwrap();
    assert_eq!(ed.project.clip(v2).unwrap().speed, 1.5);
    run(&mut ed, "clip.detach_audio", &json!({"ids": [v]})).unwrap();
    assert_eq!(ed.project.linked_peer(v), None);
    assert_eq!(ed.project.linked_peer(a), None);
    run(&mut ed, "clip.move", &json!({"id": a, "at": 0})).unwrap();
    assert_eq!(ed.project.clip(v).unwrap().start, 3.0);
    assert_eq!(ed.project.clip(a).unwrap().start, 0.0);
    // второй раз на том же видеоклипе — ошибка
    run(&mut ed, "clip.detach_audio", &json!({"ids": [v]})).unwrap_err();
    // режим без авторазделения (audio: false) + последующее ручное отделение звука
    let mut ed2 = ed_with_media(10.0);
    run(&mut ed2, "clip.add", &json!({"media": 3, "offset": 1, "duration": 4, "audio": false})).unwrap();
    assert!(ed2.project.tracks[1].clips.is_empty());
    let v0 = ed2.project.tracks[0].clips[0].id;
    assert_eq!(ed2.project.clip(v0).unwrap().volume, 1.0);
    run(&mut ed2, "clip.detach_audio", &json!({"ids": [v0]})).unwrap();
    let au: Vec<_> = ed2.project.tracks.iter().filter(|t| t.kind == crate::doc::TrackKind::Audio).flat_map(|t| &t.clips).collect();
    assert_eq!(au.len(), 1);
    assert_eq!((au[0].start, au[0].duration, au[0].offset), (0.0, 4.0, 1.0));
    assert_eq!(ed2.project.clip(v0).unwrap().volume, 0.0);
    run(&mut ed2, "clip.detach_audio", &json!({"ids": [v0]})).unwrap_err();
    // видео без звука не создаёт аудиоклип
    let mut ed3 = Editor::new();
    let mid = ed3.project.alloc();
    ed3.project.media.push(Media { id: mid, path: "/x/mute.mp4".into(), kind: MediaKind::Video, duration: 5.0, width: 1920, height: 1080, fps: 30.0, has_audio: false, transcript: None });
    run(&mut ed3, "clip.add", &json!({"media": mid})).unwrap();
    assert_eq!(ed3.project.tracks[0].clips.len(), 1);
    assert!(ed3.project.tracks[1].clips.is_empty());
}

#[test]
fn track_move_order() {
    let mut ed = Editor::new();
    for _ in 0..3 {
        run(&mut ed, "track.add", &json!({"kind": "video"})).unwrap();
    }
    let ids: Vec<u64> = ed.project.tracks.iter().filter(|t| t.kind == crate::doc::TrackKind::Video).map(|t| t.id).collect();
    // сверху вниз сейчас: последняя, средняя, первая; первую — наверх
    run(&mut ed, "track.move", &json!({"id": ids[0], "to": 0})).unwrap();
    let now: Vec<u64> = ed.project.tracks.iter().filter(|t| t.kind == crate::doc::TrackKind::Video).map(|t| t.id).collect();
    let mut want = ids[1..].to_vec();
    want.push(ids[0]);
    assert_eq!(now, want);
    run(&mut ed, "track.move", &json!({"id": ids[0], "by": 1})).unwrap();
    let now: Vec<u64> = ed.project.tracks.iter().filter(|t| t.kind == crate::doc::TrackKind::Video).map(|t| t.id).collect();
    let n = want.len();
    want.swap(n - 1, n - 2);
    assert_eq!(now, want);
}

#[test]
fn keyframes() {
    let mut ed = ed_with_media(10.0);
    run(&mut ed, "clip.add", &json!({"media": 3, "duration": 4})).unwrap();
    let c = ed.project.tracks[0].clips[0].id;
    // масштаб 1 → 2 за 2 с, эффект transform создаётся сам
    run(&mut ed, "key.set", &json!({"id": c, "param": "transform.scale", "at": 0, "value": 1})).unwrap();
    run(&mut ed, "key.set", &json!({"id": c, "param": "transform.scale", "at": 2, "value": 2})).unwrap();
    let clip = ed.project.clip(c).unwrap().clone();
    let eid = clip.effects.iter().find(|e| e.kind == "transform").unwrap().id;
    let path = format!("e{eid}.scale");
    assert_eq!(clip.value_at(&path, 1.0).unwrap().as_f64().unwrap(), 1.5);
    assert_eq!(clip.value_at(&path, 3.0).unwrap().as_f64().unwrap(), 2.0);
    let at = clip.at(0.5);
    assert_eq!(at.effects[0].params["scale"].as_f64().unwrap(), 1.25);
    // разрез в 1 с: справа ключи сдвинуты, эффект с новым id
    run(&mut ed, "clip.split", &json!({"at": 1})).unwrap();
    let right = ed.project.tracks[0].clips[1].clone();
    let reid = right.effects[0].id;
    assert_ne!(reid, eid);
    let rk = &right.keys[&format!("e{reid}.scale")];
    assert_eq!(rk[0].t, -1.0);
    assert_eq!(right.value_at(&format!("e{reid}.scale"), 0.0).unwrap().as_f64().unwrap(), 1.5);
    // ошибка — без шага в истории
    let undo = ed.undo_label().map(str::to_string);
    run(&mut ed, "key.set", &json!({"id": c, "param": "transform.bogus"})).unwrap_err();
    assert_eq!(ed.undo_label().map(str::to_string), undo);
    // удаление эффекта убирает его ключи
    run(&mut ed, "effect.remove", &json!({"id": eid})).unwrap();
    assert!(ed.project.clip(c).unwrap().keys.is_empty());
}

#[test]
fn keyframe_color_interp() {
    let k = vec![crate::doc::Key { t: 0.0, v: json!("#000000"), ease: "linear".into() }, crate::doc::Key { t: 1.0, v: json!("#ff0080"), ease: "linear".into() }];
    assert_eq!(crate::doc::interp(&k, 0.5), json!("#800040"));
    let h = vec![crate::doc::Key { t: 0.0, v: json!(0.0), ease: "hold".into() }, crate::doc::Key { t: 1.0, v: json!(1.0), ease: "linear".into() }];
    assert_eq!(crate::doc::interp(&h, 0.9), json!(0.0));
}

#[test]
fn eases_and_key_move() {
    use crate::doc::ease;
    for k in ["linear", "ease", "in", "out", "back", "elastic", "bounce", "bezier(0.25,0.1,0.25,1)"] {
        assert!((ease(k, 0.0)).abs() < 1e-6, "{k} в 0");
        assert!((ease(k, 1.0) - 1.0).abs() < 1e-6, "{k} в 1");
    }
    assert!(ease("back", 0.8) > 1.0, "перелёт");
    // bezier(0,0,1,1) — прямая
    assert!((ease("bezier(0,0,1,1)", 0.3) - 0.3).abs() < 1e-3);
    let mut ed = ed_with_media(10.0);
    run(&mut ed, "clip.add", &json!({"media": 3, "duration": 4})).unwrap();
    let c = ed.project.tracks[0].clips[0].id;
    run(&mut ed, "key.set", &json!({"id": c, "param": "transform.x", "at": 0, "value": 0, "ease": "bezier(0.4,0,0.2,1)"})).unwrap();
    run(&mut ed, "key.set", &json!({"id": c, "param": "transform.x", "at": 2, "value": 1})).unwrap();
    run(&mut ed, "key.set", &json!({"id": c, "param": "transform.x", "at": 0, "ease": "nonsense"})).unwrap_err();
    // value без ease — сглаживание прежнее
    run(&mut ed, "key.set", &json!({"id": c, "param": "transform.x", "at": 0, "value": 0.1})).unwrap();
    let path = ed.project.clip(c).unwrap().keys.keys().next().unwrap().clone();
    assert_eq!(ed.project.clip(c).unwrap().keys[&path][0].ease, "bezier(0.4,0,0.2,1)");
    run(&mut ed, "key.move", &json!({"id": c, "param": "transform.x", "from": 2, "to": 3, "value": 0.5})).unwrap();
    let k = &ed.project.clip(c).unwrap().keys[&path][1];
    assert_eq!((k.t, k.v.as_f64().unwrap()), (3.0, 0.5));
    run(&mut ed, "key.move", &json!({"id": c, "param": "transform.x", "from": 3, "to": 0})).unwrap_err();
}

#[test]
fn title_animation_style_fields() {
    let mut ed = Editor::new();
    let out = run(&mut ed, "title.add", &json!({"text": "Привет мир\nВторая", "style": {"by": "word", "in": "scatter", "out": "type", "anim": 0.8, "stagger": 0.7, "ease": "back"}})).unwrap();
    let id = out.data["id"].as_u64().unwrap();
    run(&mut ed, "clip.set", &json!({"id": id, "style": {"by": "char", "in": "type"}})).unwrap();
    let crate::doc::Source::Title { style, .. } = &ed.project.clip(id).unwrap().source else { panic!("ожидался титр") };
    assert_eq!(style["by"], "char");
    assert_eq!(style["in"], "type");
    assert_eq!(style["out"], "type");
    assert_eq!(style["anim"], 0.8);
    assert_eq!(style["stagger"], 0.7);
    assert_eq!(style["ease"], "back");
}

#[test]
fn motion_presets_apply_combine_and_undo() {
    let mut ed = ed_with_media(10.0);
    run(&mut ed, "clip.add", &json!({"media": 3, "duration": 4})).unwrap();
    let cid = ed.project.tracks[0].clips[0].id;
    let listed = run(&mut ed, "motion.list", &json!({})).unwrap();
    assert!(listed.text.contains("slide_in_left") && listed.text.contains("ken_burns_in"));

    // вход + выход на одном клипе сочетаются
    run(&mut ed, "motion.apply", &json!({"preset": "slide_in_left", "ids": [cid], "duration": 0.5})).unwrap();
    run(&mut ed, "motion.apply", &json!({"preset": "zoom_out_exit", "ids": [cid], "duration": 0.5})).unwrap();
    let c = ed.project.clip(cid).unwrap();
    let eid = c.effects.iter().find(|e| e.kind == "transform").unwrap().id;
    let at_start = c.at(0.0);
    assert!(at_start.effects[0].params["x"].as_f64().unwrap() < -0.5);
    assert_eq!(at_start.effects[0].params["opacity"].as_f64().unwrap(), 0.0);
    let at_mid = c.at(2.0);
    assert_eq!(at_mid.effects[0].params["x"].as_f64().unwrap(), 0.0);
    assert_eq!(at_mid.effects[0].params["opacity"].as_f64().unwrap(), 1.0);
    let at_end = c.at(4.0);
    assert!(at_end.effects[0].params["scale"].as_f64().unwrap() < 0.5);
    assert_eq!(at_end.effects[0].params["opacity"].as_f64().unwrap(), 0.0);

    // другой вход заменяет прежний вход, но сохраняет выход
    run(&mut ed, "motion.apply", &json!({"preset": "pop_in", "ids": [cid], "duration": 0.5})).unwrap();
    let c = ed.project.clip(cid).unwrap();
    assert!(!c.keys.contains_key(&format!("e{eid}.x")), "старый вход убран");
    assert_eq!(c.at(4.0).effects[0].params["opacity"].as_f64().unwrap(), 0.0, "выход сохранён");

    // движение по всему клипу (pan_left) ставит постоянный масштаб и ключи x
    run(&mut ed, "motion.apply", &json!({"preset": "pan_left", "ids": [cid]})).unwrap();
    let c = ed.project.clip(cid).unwrap();
    assert!(c.effects[0].params["scale"].as_f64().unwrap() > 1.1);
    assert!(c.at(0.0).effects[0].params["x"].as_f64().unwrap() > 0.0);
    assert!(c.at(4.0).effects[0].params["x"].as_f64().unwrap() < 0.0);

    // отмена возвращает предыдущий шаг
    run(&mut ed, "edit.undo", &json!({})).unwrap();
    assert!(ed.project.clip(cid).unwrap().keys.contains_key(&format!("e{eid}.scale")));
}

#[test]
fn layers_shapes_masks_blend_and_parent() {
    let mut ed = Editor::new();
    let p_out = run(&mut ed, "shape.add", &json!({"kind": "rect", "at": 0, "duration": 4, "style": {"fill": "#ff4400", "width": 0.4, "height": 0.4}})).unwrap();
    let parent_id = p_out.data["id"].as_u64().unwrap();
    let c_out = run(&mut ed, "shape.add", &json!({"kind": "star", "at": 0, "duration": 4, "track": 1, "style": {"fill": "#00ccff", "points": 6}})).unwrap();
    let child_id = c_out.data["id"].as_u64().unwrap();

    // наложение и маска с ключевыми кадрами
    run(&mut ed, "clip.set", &json!({"id": child_id, "blend": "screen", "parent": parent_id})).unwrap();
    run(&mut ed, "clip.set", &json!({"id": child_id, "blend": "bogus"})).unwrap_err();
    run(&mut ed, "effect.add", &json!({"ids": [child_id], "kind": "mask", "params": {"shape": "ellipse", "width": 0.5, "height": 0.5, "feather": 0.05}})).unwrap();
    run(&mut ed, "key.set", &json!({"id": child_id, "param": "mask.width", "at": 0, "value": 0.3})).unwrap();
    run(&mut ed, "key.set", &json!({"id": child_id, "param": "mask.width", "at": 2, "value": 0.9})).unwrap();
    // ключи стиля фигуры
    run(&mut ed, "key.set", &json!({"id": parent_id, "param": "shape.width", "at": 0, "value": 0.2})).unwrap();
    run(&mut ed, "key.set", &json!({"id": parent_id, "param": "shape.width", "at": 2, "value": 0.6})).unwrap();

    // защита от циклов родитель → потомок
    run(&mut ed, "clip.set", &json!({"id": parent_id, "parent": parent_id})).unwrap_err();
    run(&mut ed, "clip.set", &json!({"id": parent_id, "parent": child_id})).unwrap_err();

    // наследование трансформации (сдвиг, масштаб, поворот, прозрачность)
    run(&mut ed, "effect.add", &json!({"ids": [parent_id], "kind": "transform", "params": {"x": 0.2, "y": 0.1, "scale": 2.0, "rotation": 90.0, "opacity": 0.5}})).unwrap();
    run(&mut ed, "effect.add", &json!({"ids": [child_id], "kind": "transform", "params": {"x": 0.1, "y": 0.0, "scale": 0.5, "rotation": 15.0, "opacity": 0.8}})).unwrap();
    let ch = ed.project.clip(child_id).unwrap();
    let wxf = crate::render::compose::world_transform(&ed.project, &ch.at(1.0), 1.0);
    // при повороте родителя на 90° локальный x=+0.1*W*2 уходит в +y
    assert!((wxf.x - 0.2).abs() < 1e-3, "wxf.x={}", wxf.x);
    assert!(wxf.y > 0.1 + 0.1, "wxf.y={}", wxf.y);
    assert!((wxf.scale - 1.0).abs() < 1e-4);
    assert!((wxf.rot - 105.0).abs() < 1e-3);
    assert!((wxf.opacity - 0.4).abs() < 1e-6);

    // эффект матового стекла (glassmorphism) с ключами
    run(&mut ed, "effect.add", &json!({"ids": [parent_id], "kind": "glass", "params": {"blur": 20, "tint": "#F9EFEF", "tint_opacity": 0.18, "border_opacity": 0.5, "highlight": 0.32, "shadow": 0.28}})).unwrap();
    run(&mut ed, "key.set", &json!({"id": parent_id, "param": "glass.blur", "at": 0, "value": 4})).unwrap();
    run(&mut ed, "key.set", &json!({"id": parent_id, "param": "glass.blur", "at": 2, "value": 24})).unwrap();
    let g_at1 = ed.project.clip(parent_id).unwrap().at(1.0);
    let g_ef = g_at1.effects.iter().find(|e| e.kind == "glass").unwrap();
    assert!((g_ef.params["blur"].as_f64().unwrap() - 14.0).abs() < 1e-4);

    // удаление родителя отвязывает потомка
    run(&mut ed, "clip.remove", &json!({"ids": [parent_id]})).unwrap();
    assert_eq!(ed.project.clip(child_id).unwrap().parent, None);
}

#[test]
fn template_params_set_keys_and_reset() {
    let dir = std::env::temp_dir().join(format!("syn_ve_c3_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let tpl_path = dir.join("DemoMacro.setting");
    let setting = r#"{
        Tools = ordered() {
            DemoMacro = MacroOperator {
                Inputs = ordered() {
                    Input1 = InstanceInput { SourceOp = "Master", Source = "Center", Name = "Position", },
                    Input2 = InstanceInput { SourceOp = "Master", Source = "Size", Default = 1, },
                    Input3 = InstanceInput { SourceOp = "Bg", Source = "Type", },
                    Input4 = InstanceInput { SourceOp = "Bg", Source = "TopLeftRed", Name = "Color", ControlGroup = 3, Default = 0, },
                    Input5 = InstanceInput { SourceOp = "Bg", Source = "TopLeftGreen", ControlGroup = 3, Default = 0, },
                    Input6 = InstanceInput { SourceOp = "Bg", Source = "TopLeftBlue", ControlGroup = 3, Default = 1, },
                    Input7 = InstanceInput { SourceOp = "Bg", Source = "TopLeftAlpha", ControlGroup = 3, Default = 1, },
                    Input8 = InstanceInput { SourceOp = "Title1", Source = "StyledText", },
                    Input9 = InstanceInput { SourceOp = "Title1", Source = "Red1Clone", Name = "Color", ControlGroup = 5, Default = 1, },
                    Input10 = InstanceInput { SourceOp = "Title1", Source = "Green1Clone", ControlGroup = 5, Default = 1, },
                    Input11 = InstanceInput { SourceOp = "Title1", Source = "Blue1Clone", ControlGroup = 5, Default = 1, },
                },
                Outputs = { MainOutput1 = InstanceOutput { SourceOp = "M", Source = "Output", }, },
                Tools = ordered() {
                    Bg = Background { Inputs = { TopLeftRed = Input { Value = 0, }, TopLeftGreen = Input { Value = 0, }, TopLeftBlue = Input { Value = 1, }, TopLeftAlpha = Input { Value = 1, }, }, },
                    Title1 = TextPlus { Inputs = { StyledText = Input { Value = "DEFAULT", }, Size = Input { Value = 0.12, }, Red1 = Input { Value = 1, }, Green1 = Input { Value = 1, }, Blue1 = Input { Value = 1, }, }, },
                    Master = Transform { Inputs = { Input = Input { SourceOp = "Title1", Source = "Output", }, Size = Input { Value = 1, }, Center = Input { Value = { 0.5, 0.5 }, }, }, },
                    M = Merge { Inputs = { Background = Input { SourceOp = "Bg", Source = "Output", }, Foreground = Input { SourceOp = "Master", Source = "Output", }, }, },
                },
            },
        },
    }"#;
    std::fs::write(&tpl_path, setting).unwrap();

    let mut ed = Editor::new();
    let add = run(&mut ed, "title.add", &json!({"text": "DEFAULT", "at": 0, "duration": 4, "style": {"template": tpl_path.display().to_string()}})).unwrap();
    let cid = add.data["id"].as_u64().unwrap();

    // template.params возвращает сгруппированные параметры MacroOperator
    let p_out = run(&mut ed, "template.params", &json!({"id": cid})).unwrap();
    let plist = p_out.data["params"].as_array().unwrap();
    let keys: Vec<&str> = plist.iter().map(|v| v["key"].as_str().unwrap()).collect();
    assert_eq!(keys, vec!["Master.Center", "Master.Size", "Bg.Type", "Bg.TopLeftColor", "Title1.StyledText", "Title1.Color1"]);

    // template.set правит цвет, точку и текст + проверяет ошибки
    run(&mut ed, "template.set", &json!({"id": cid, "params": {"Bg.TopLeftColor": "#00ff00", "Master.Center": [0.25, 0.75], "Title1.StyledText": "CUSTOM"}})).unwrap();
    run(&mut ed, "template.set", &json!({"id": cid, "param": "Bg.TopLeftColor", "value": "not-a-color"})).unwrap_err();
    run(&mut ed, "template.set", &json!({"id": cid, "param": "NoSuch.Param", "value": 1})).unwrap_err();

    // ключевые кадры на параметре шаблона (tpl.Master.Size) + интерполяция через Clip::at и FrameSource
    run(&mut ed, "key.set", &json!({"id": cid, "param": "tpl.Master.Size", "at": 0, "value": 0.5})).unwrap();
    run(&mut ed, "key.set", &json!({"id": cid, "param": "tpl.Master.Size", "at": 2, "value": 1.5})).unwrap();
    let at1 = ed.project.clip(cid).unwrap().at(1.0);
    let crate::doc::Source::Title { style, .. } = &at1.source else { panic!("title") };
    assert!((style["p:Master.Size"].as_f64().unwrap() - 1.0).abs() < 1e-4);

    // FrameSource рендерит с учётом p:Bg.TopLeftColor = #00ff00 (зелёный фон!)
    let mut fs = crate::render::compose::FrameSource::new(64, 64);
    let frames = fs.frames(&ed.project, 1.0);
    let fr = frames.get(&cid).expect("rendered fusion frame");
    assert!(fr.rgba[1] > 200 && fr.rgba[0] < 30 && fr.rgba[2] < 30, "expected green bg, got {:?}", &fr.rgba[0..4]);

    // Сброс параметров возвращает исходный синий фон и снимает ключи шаблона
    run(&mut ed, "template.set", &json!({"id": cid, "reset": true})).unwrap();
    assert!(ed.project.clip(cid).unwrap().keys.is_empty());
    let frames2 = fs.frames(&ed.project, 1.0);
    let fr2 = frames2.get(&cid).expect("rendered reset frame");
    assert!(fr2.rgba[2] > 200 && fr2.rgba[1] < 30, "expected blue bg after reset, got {:?}", &fr2.rgba[0..4]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn project_custom_dimensions_and_bg() {
    let mut ed = Editor::new();
    assert_eq!(ed.project.settings.bg, "#000000");
    run(&mut ed, "project.settings", &json!({"width": 1441, "height": 2560, "bg": "#1e293b"})).unwrap();
    // чётное выравнивание для кодеков
    assert_eq!(ed.project.settings.width, 1440);
    assert_eq!(ed.project.settings.height, 2560);
    assert_eq!(ed.project.settings.bg, "#1e293b");
    run(&mut ed, "project.settings", &json!({"bg": "transparent"})).unwrap();
    assert_eq!(ed.project.settings.bg, "transparent");
    run(&mut ed, "project.settings", &json!({"bg": "not-a-color"})).unwrap_err();
    run(&mut ed, "project.settings", &json!({"width": 8})).unwrap_err();
}

#[test]
fn template_gallery_filter_preview_and_apply_by_path() {
    let dir = std::env::temp_dir().join(format!("sve-c4-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let sub1 = dir.join("Титры/Заголовки");
    let sub2 = dir.join("Графика/Фоны");
    std::fs::create_dir_all(&sub1).unwrap();
    std::fs::create_dir_all(&sub2).unwrap();

    let tpl_title = sub1.join("Neon_Banner_01.setting");
    let tpl_bg = sub2.join("Sunset_Wave.setting");
    let lua_src = r#"{
        Tools = ordered() {
            Bg = Background {
                Inputs = {
                    TopLeftRed = Input { Value = 0.9, },
                    TopLeftGreen = Input { Value = 0.3, },
                    TopLeftBlue = Input { Value = 0.1, },
                    TopLeftAlpha = Input { Value = 1.0, },
                },
            },
            Title = TextPlus {
                Inputs = {
                    StyledText = Input { Value = "HELLO", },
                    Size = Input { Value = 0.12, },
                },
            },
            Merge1 = Merge {
                Inputs = {
                    Background = Input { SourceOp = "Bg", Source = "Output", },
                    Foreground = Input { SourceOp = "Title", Source = "Output", },
                },
            },
        },
    }"#;
    std::fs::write(&tpl_title, lua_src).unwrap();
    std::fs::write(&tpl_bg, lua_src).unwrap();

    // Сканирование и фильтрация по поиску, верхнему разделу и подразделу
    let entries = crate::ui::templates::scan_catalog(&dir);
    assert_eq!(entries.len(), 2);
    assert_eq!(crate::ui::templates::filter_catalog(&entries, "neon", "", "").len(), 1);
    assert_eq!(crate::ui::templates::filter_catalog(&entries, "", "Титры", "Заголовки").len(), 1);
    assert_eq!(crate::ui::templates::filter_catalog(&entries, "wave", "Графика", "Фоны").len(), 1);
    assert_eq!(crate::ui::templates::filter_catalog(&entries, "wave", "Титры", "").len(), 0);

    // Генерация превью-кадра и 12-кадровой анимированной ленты
    let hero = crate::ui::templates::render_hero(&tpl_title).expect("hero frame");
    assert_eq!((hero.width, hero.height), (crate::ui::templates::THUMB_W, crate::ui::templates::THUMB_H));
    assert!(hero.rgba[0] > 180 && hero.rgba[3] == 255, "expected warm hero backdrop, got {:?}", &hero.rgba[0..4]);

    let strip = crate::ui::templates::render_strip(&tpl_title).expect("hover strip");
    assert_eq!(strip.len(), crate::ui::templates::STRIP_FRAMES);
    assert_eq!(strip[0].width, crate::ui::templates::THUMB_W);

    // Наложение шаблона по точному пути (как делает перенос из галереи)
    let mut ed = Editor::new();
    let out = run(&mut ed, "template.apply", &json!({"name": "Neon_Banner_01", "path": tpl_title.display().to_string(), "at": 1.5, "duration": 3.0})).unwrap();
    let cid = out.data["id"].as_u64().unwrap();
    let clip = ed.project.clip(cid).unwrap();
    assert!((clip.start - 1.5).abs() < 1e-4);
    assert!((clip.duration - 3.0).abs() < 1e-4);
    let crate::doc::Source::Title { style, .. } = &clip.source else { panic!("expected Title") };
    assert_eq!(style["template"].as_str(), Some(tpl_title.to_str().unwrap()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fusion_transition_set_render_and_params() {
    let dir = std::env::temp_dir().join(format!("sve-c5-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Синтетический переход Fusion с MainInput1/MainInput2 (MacroOperator), InvertTransform и Dissolve
    let tr_path = dir.join("Demo_Blend_Transition.setting");
    let tr_lua = r#"{
        Tools = ordered() {
            DemoTrans = MacroOperator {
                Inputs = ordered() {
                    MainInput1 = InstanceInput { SourceOp = "TxA", Source = "Input", },
                    MainInput2 = InstanceInput { SourceOp = "TxB", Source = "Input", },
                    Input1 = InstanceInput { SourceOp = "TxA", Source = "Size", Name = "Zoom", Default = 1.0, },
                },
                Outputs = { MainOutput1 = InstanceOutput { SourceOp = "Mix1", Source = "Output", }, },
                Tools = ordered() {
                    TxA = Transform {
                        Inputs = {
                            Size = Input { Value = 1.0, },
                            InvertTransform = Input { Value = 1, },
                        },
                    },
                    TxB = Transform {
                        Inputs = {
                            Size = Input { Value = 1.0, },
                        },
                    },
                    Mix1 = Dissolve {
                        Inputs = {
                            Background = Input { SourceOp = "TxA", Source = "Output", },
                            Foreground = Input { SourceOp = "TxB", Source = "Output", },
                            Mix = Input { SourceOp = "MixCurve", Source = "Value", },
                        },
                    },
                    MixCurve = BezierSpline {
                        KeyFrames = {
                            [0] = { 0, RH = { 10, 0.33 }, Flags = { Linear = true } },
                            [30] = { 1, LH = { 20, 0.66 }, Flags = { Linear = true } },
                        },
                    },
                },
            },
        },
    }"#;
    std::fs::write(&tr_path, tr_lua).unwrap();

    // Проверяем разбор MainInput1/MainInput2 и is_transition()
    let comp = crate::fusion::load_cached(&tr_path).unwrap();
    assert!(comp.is_transition());
    assert_eq!(comp.main_inputs.len(), 2);

    // Ставим два клипа встык (красный 0..2с и синий 2..4с)
    let mut ed = Editor::new();
    let v1 = ed.project.tracks[0].id;
    let c1 = ed.project.alloc();
    let c2 = ed.project.alloc();
    ed.project.tracks[0].clips.push(crate::doc::Clip {
        id: c1,
        source: crate::doc::Source::Color { color: "#ff0000".into() },
        start: 0.0,
        duration: 2.0,
        offset: 0.0,
        speed: 1.0,
        volume: 1.0,
        name: "A".into(),
        effects: Vec::new(),
        transition: None,
        keys: Default::default(),
        blend: "normal".into(),
        parent: None,
        link: None,
    });
    ed.project.tracks[0].clips.push(crate::doc::Clip {
        id: c2,
        source: crate::doc::Source::Color { color: "#0000ff".into() },
        start: 2.0,
        duration: 2.0,
        offset: 0.0,
        speed: 1.0,
        volume: 1.0,
        name: "B".into(),
        effects: Vec::new(),
        transition: None,
        keys: Default::default(),
        blend: "normal".into(),
        parent: None,
        link: None,
    });
    let _ = v1;

    // Ставим Fusion-переход через transition.set
    run(&mut ed, "transition.set", &json!({
        "id": c2,
        "kind": "fusion",
        "template": tr_path.display().to_string(),
        "duration": 1.0
    })).unwrap();
    let tr = ed.project.clip(c2).unwrap().transition.as_ref().unwrap();
    assert!(tr.is_fusion());
    assert_eq!(tr.title(), "Demo Blend Transition");
    assert!((tr.duration - 1.0).abs() < 1e-4);

    // Рендер через FrameSource: на t=2.0 (середина перехода, q=0.5) красный и синий смешаны (~127, 0, ~127)
    let mut fs = crate::render::compose::FrameSource::new(320, 180);
    let mid = fs.frames(&ed.project, 2.0);
    let fr = mid.get(&c2).expect("fusion transition frame at midpoint");
    let px = ((fr.height / 2 * fr.width + fr.width / 2) * 4) as usize;
    let (r, g, b, a) = (fr.rgba[px], fr.rgba[px + 1], fr.rgba[px + 2], fr.rgba[px + 3]);
    assert!(r > 90 && r < 165 && b > 90 && b < 165 && g < 20 && a > 240, "expected blended red+blue at midpoint, got ({r},{g},{b},{a})");

    // Обновляем параметр MacroOperator и длительность без повторного указания template
    run(&mut ed, "transition.set", &json!({
        "id": c2,
        "kind": "fusion",
        "duration": 1.2,
        "params": {"TxA.Size": 1.5}
    })).unwrap();
    let tr2 = ed.project.clip(c2).unwrap().transition.as_ref().unwrap();
    assert!(tr2.is_fusion());
    assert!((tr2.duration - 1.2).abs() < 1e-4);
    assert_eq!(tr2.params.get("p:TxA.Size").and_then(|v| v.as_f64()), Some(1.5));

    // Сброс параметров
    run(&mut ed, "transition.set", &json!({"id": c2, "kind": "fusion", "reset": true})).unwrap();
    assert!(ed.project.clip(c2).unwrap().transition.as_ref().unwrap().params.is_empty());

    // Превью перехода в галерее (render_hero с MainInput1/MainInput2)
    let hero = crate::ui::templates::render_hero(&tr_path).expect("transition hero");
    assert_eq!((hero.width, hero.height), (crate::ui::templates::THUMB_W, crate::ui::templates::THUMB_H));

    // transition.list работает всегда (минимум встроенные переходы)
    let list_out = run(&mut ed, "transition.list", &json!({"query": "zoom"})).unwrap();
    assert!(list_out.data["builtin"].as_array().unwrap().iter().any(|x| x["kind"] == "zoom"));

    let _ = std::fs::remove_dir_all(&dir);
}
