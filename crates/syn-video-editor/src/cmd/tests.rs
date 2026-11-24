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
    // копия двух клипов в 10 с: та же дорожка, взаимное расположение сохранено
    run(&mut ed, "clip.copy", &json!({})).unwrap();
    run(&mut ed, "clip.paste", &json!({"at": 10})).unwrap();
    assert_eq!(ed.selection.len(), 2);
    let starts: Vec<f64> = ed.selection.iter().map(|id| ed.project.clip(*id).unwrap().start).collect();
    assert_eq!(starts, vec![10.0, 12.0]);
    let n = ed.project.tracks.len();
    assert!(ed.selection.iter().all(|id| ed.project.find_clip(*id).map(|f| f.0) == Some(0)));
    // вставка поверх занятого — на новую дорожку
    run(&mut ed, "clip.paste", &json!({"at": 0})).unwrap();
    assert_eq!(ed.project.tracks.len(), n + 1);
    // удалить выделенное
    run(&mut ed, "clip.remove", &json!({})).unwrap();
    assert!(ed.project.tracks[n].clips.is_empty());
}

#[test]
fn detach_audio() {
    let mut ed = ed_with_media(10.0);
    run(&mut ed, "clip.add", &json!({"media": 3, "offset": 1, "duration": 4})).unwrap();
    let v = ed.project.tracks.iter().flat_map(|t| &t.clips).next().unwrap().id;
    run(&mut ed, "clip.detach_audio", &json!({"ids": [v]})).unwrap();
    let au: Vec<_> = ed.project.tracks.iter().filter(|t| t.kind == crate::doc::TrackKind::Audio).flat_map(|t| &t.clips).collect();
    assert_eq!(au.len(), 1);
    assert_eq!((au[0].start, au[0].duration, au[0].offset), (0.0, 4.0, 1.0));
    assert_eq!(ed.project.clip(v).unwrap().volume, 0.0);
    // второй раз — ошибка
    run(&mut ed, "clip.detach_audio", &json!({"ids": [v]})).unwrap_err();
}
