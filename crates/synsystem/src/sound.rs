//! Звук: устройства и потоки PipeWire (`pw-dump`, `wpctl`) и функции телефона через palaudiod
//! (synmobile, docs/14-audio.md): маршрут динамик/наушники, Dolby Atmos и его профили.
//!
//! Всё блокирующее — звать из фонового потока. Громкость «в процентах» — как у `wpctl`
//! (кубический корень из линейного множителя PipeWire).

use crate::util::{output, which};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::Duration;

/// Вид узла PipeWire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// Выход (Audio/Sink).
    Output,
    /// Вход (Audio/Source).
    Input,
    /// Поток программы (Stream/Output/Audio).
    AppStream,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: u32,
    pub kind: NodeKind,
    /// `node.name` — устойчивое имя (для выбора по умолчанию).
    pub name: String,
    /// Человеческое имя: описание устройства или название программы.
    pub label: String,
    /// Значок (`device.icon-name` / `application.icon-name`), если есть.
    pub icon: Option<String>,
    /// Проценты (как у wpctl), `None` — узел громкость не сообщил.
    pub percent: Option<u32>,
    pub muted: bool,
    /// Выход/вход по умолчанию.
    pub default: bool,
    /// Узел сейчас играет/пишет.
    pub running: bool,
}

/// Узлы PipeWire (выходы, входы, потоки программ). `None` — нет PipeWire.
pub fn nodes() -> Option<Vec<Node>> {
    if !which("pw-dump") {
        return None;
    }
    let text = output("pw-dump", &[])?;
    Some(parse_pw_dump(&text))
}

/// Разбор вывода `pw-dump` (отдельно — для тестов).
pub fn parse_pw_dump(text: &str) -> Vec<Node> {
    let Ok(Value::Array(objs)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut def_sink = String::new();
    let mut def_source = String::new();
    for o in &objs {
        if o["type"] != "PipeWire:Interface:Metadata" || o["props"]["metadata.name"] != "default" {
            continue;
        }
        for m in o["metadata"].as_array().into_iter().flatten() {
            let name = m["value"]["name"].as_str().unwrap_or_default().to_string();
            match m["key"].as_str() {
                Some("default.audio.sink") => def_sink = name,
                Some("default.audio.source") => def_source = name,
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for o in &objs {
        if o["type"] != "PipeWire:Interface:Node" {
            continue;
        }
        let info = &o["info"];
        let p = &info["props"];
        let kind = match p["media.class"].as_str() {
            Some("Audio/Sink") => NodeKind::Output,
            Some("Audio/Source") => NodeKind::Input,
            Some("Stream/Output/Audio") => NodeKind::AppStream,
            _ => continue,
        };
        let s = |k: &str| p[k].as_str().map(str::to_string);
        let name = s("node.name").unwrap_or_default();
        let label = match kind {
            NodeKind::AppStream => s("application.name").or_else(|| s("media.name")).or_else(|| s("node.name")),
            _ => s("node.description").or_else(|| s("node.nick")).or_else(|| s("node.name")),
        }
        .unwrap_or_default();
        let icon = s("device.icon-name").or_else(|| s("application.icon-name"));
        let props = info["params"]["Props"].as_array().and_then(|a| a.first());
        let (percent, muted) = match props {
            Some(pr) => {
                let vols: Vec<f64> =
                    pr["channelVolumes"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
                let lin = if vols.is_empty() { pr["volume"].as_f64() } else { Some(vols.iter().sum::<f64>() / vols.len() as f64) };
                (lin.map(|v| (v.cbrt() * 100.0).round() as u32), pr["mute"].as_bool().unwrap_or(false))
            }
            None => (None, false),
        };
        let default = match kind {
            NodeKind::Output => name == def_sink,
            NodeKind::Input => name == def_source,
            NodeKind::AppStream => false,
        };
        let running = info["state"] == "running";
        out.push(Node { id: o["id"].as_u64().unwrap_or(0) as u32, kind, name, label, icon, percent, muted, default, running });
    }
    out
}

/// Выход или вход по умолчанию.
pub fn set_default(id: u32) {
    let _ = Command::new("wpctl").args(["set-default", &id.to_string()]).status();
}

/// Громкость узла в процентах (≤ `max`).
pub fn set_node_volume(id: u32, percent: u32, max: u32) {
    let p = percent.min(max);
    let _ = Command::new("wpctl").args(["set-volume", &id.to_string(), &format!("{p}%")]).status();
}

pub fn set_node_mute(id: u32, mute: bool) {
    let _ = Command::new("wpctl").args(["set-mute", &id.to_string(), if mute { "1" } else { "0" }]).status();
}

// --- телефон: palaudiod ---------------------------------------------------------------------------

const PALAUDIO_CONTROL: &str = "/run/palaudio/control";

/// Куда выводить звук телефона.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Наушники, если вставлены, иначе динамики.
    Auto,
    Speaker,
    Headphones,
}

impl Route {
    pub fn as_str(self) -> &'static str {
        match self {
            Route::Auto => "auto",
            Route::Speaker => "speaker",
            Route::Headphones => "headphones",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhoneAudio {
    /// Текущий выход: `speaker`, `headphones`, `headset`.
    pub output: String,
    pub route: Route,
    pub headphones_plugged: bool,
    pub headset_mic: bool,
    pub playing: bool,
    pub recording: bool,
    /// `None` — Dolby на устройстве нет.
    pub dolby: Option<bool>,
    /// −1 — профиль по умолчанию.
    pub dolby_profile: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DolbyProfile {
    pub id: i32,
    /// Имя из dax-default.xml (Dynamic, Movie, Music, Custom, Voice).
    pub name: String,
}

impl DolbyProfile {
    /// Имя для людей.
    pub fn title(&self) -> String {
        match self.name.as_str() {
            "Dynamic" => "Динамический",
            "Movie" => "Кино",
            "Music" => "Музыка",
            "Custom" => "Свой",
            "Voice" => "Голос",
            n => n,
        }
        .to_string()
    }
}

fn pal_request(line: &str) -> Option<String> {
    let s = UnixStream::connect(PALAUDIO_CONTROL).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    (&s).write_all(format!("{line}\n").as_bytes()).ok()?;
    let mut r = BufReader::new(&s);
    let mut reply = String::new();
    r.read_line(&mut reply).ok()?;
    Some(reply)
}

fn parse_status(v: &Value) -> Option<PhoneAudio> {
    Some(PhoneAudio {
        output: v["output"].as_str()?.to_string(),
        route: match v["route"].as_str() {
            Some("speaker") => Route::Speaker,
            Some("headphones") => Route::Headphones,
            _ => Route::Auto,
        },
        headphones_plugged: v["jack"]["headphones"].as_bool().unwrap_or(false),
        headset_mic: v["jack"]["mic"].as_bool().unwrap_or(false),
        playing: v["playing"].as_bool().unwrap_or(false),
        recording: v["recording"].as_bool().unwrap_or(false),
        dolby: v["dolby"].as_bool(),
        dolby_profile: v["dolby_profile"].as_i64().unwrap_or(-1) as i32,
    })
}

fn pal_status_cmd(line: &str) -> Option<PhoneAudio> {
    parse_status(&serde_json::from_str(&pal_request(line)?).ok()?)
}

/// Звук телефона (palaudiod). `None` — не телефон/демона нет.
pub fn phone_audio() -> Option<PhoneAudio> {
    pal_status_cmd("status")
}

pub fn set_route(r: Route) -> Option<PhoneAudio> {
    pal_status_cmd(&format!("route {}", r.as_str()))
}

pub fn set_dolby(on: bool) -> Option<PhoneAudio> {
    pal_status_cmd(if on { "dolby on" } else { "dolby off" })
}

pub fn set_dolby_profile(id: i32) -> Option<PhoneAudio> {
    pal_status_cmd(&format!("dolby-profile {id}"))
}

pub fn dolby_profiles() -> Vec<DolbyProfile> {
    let Some(reply) = pal_request("dolby-profiles") else { return Vec::new() };
    let Ok(Value::Array(a)) = serde_json::from_str::<Value>(&reply) else { return Vec::new() };
    a.iter()
        .filter_map(|p| Some(DolbyProfile { id: p["id"].as_i64()? as i32, name: p["name"].as_str()?.to_string() }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pw_dump() {
        let text = r#"[
          {"id":30,"type":"PipeWire:Interface:Metadata","props":{"metadata.name":"default"},
           "metadata":[{"subject":0,"key":"default.audio.sink","type":"Spa:String:JSON","value":{"name":"palaudio.speaker"}},
                       {"subject":0,"key":"default.audio.source","type":"Spa:String:JSON","value":{"name":"palaudio.mic"}}]},
          {"id":53,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"media.class":"Audio/Sink",
            "node.name":"palaudio.speaker","node.description":"Встроенный звук","device.icon-name":"audio-speakers"},
            "params":{"Props":[{"volume":1.0,"mute":false,"channelVolumes":[0.729,0.729]}]}}},
          {"id":54,"type":"PipeWire:Interface:Node","info":{"state":"suspended","props":{"media.class":"Audio/Source",
            "node.name":"palaudio.mic","node.description":"Микрофон"},"params":{"Props":[{"volume":1.0,"mute":true}]}}},
          {"id":60,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"media.class":"Stream/Output/Audio",
            "node.name":"mpv","application.name":"mpv"},"params":{}}}
        ]"#;
        let n = parse_pw_dump(text);
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].kind, NodeKind::Output);
        assert_eq!(n[0].percent, Some(90));
        assert!(n[0].default && n[0].running);
        assert_eq!(n[0].icon.as_deref(), Some("audio-speakers"));
        assert!(n[1].muted && n[1].default && !n[1].running);
        assert_eq!(n[1].percent, Some(100));
        assert_eq!(n[2].label, "mpv");
        assert_eq!(n[2].percent, None);
    }

    #[test]
    fn status() {
        let v: Value = serde_json::from_str(
            r#"{"output":"speaker","route":"auto","jack":{"headphones":false,"mic":false},"playing":true,
               "recording":false,"input":"mic","dolby":true,"dolby_profile":2}"#,
        )
        .unwrap();
        let s = parse_status(&v).unwrap();
        assert_eq!(s.route, Route::Auto);
        assert_eq!(s.dolby, Some(true));
        assert_eq!(s.dolby_profile, 2);
        let v: Value = serde_json::from_str(r#"{"output":"headphones","route":"headphones","jack":{"headphones":true,"mic":true},"dolby":null}"#).unwrap();
        let s = parse_status(&v).unwrap();
        assert_eq!(s.dolby, None);
        assert!(s.headset_mic);
    }
}
