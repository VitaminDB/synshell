use synshell_common::screenrec::{self, Mode, Opts, Status};
use synshell_common::Config;
use syngui::t;

use crate::shot::Shot;

pub const HELP: &str = "synshot --record [toggle|start|stop|status|wait]
    [--screen [ВЫВОД] | --area [X,Y,Ш,В] | --window [ID]]
    [--fps 30|60] [--audio on|off] [--mic on|off] [--cursor on|off]
    [--codec h264|hevc] [--encoder auto|nvenc|vaapi|x264] [--quality high|medium|low]
    [--duration СЕКУНДЫ] [--out ФАЙЛ.mp4] [--wait]";

fn print(s: &Status) {
    println!("{}", serde_json::to_string(s).unwrap_or_default());
}

fn on(v: &str) -> Option<bool> {
    match v {
        "1" | "on" | "yes" | "true" | "вкл" => Some(true),
        "0" | "off" | "no" | "false" | "выкл" => Some(false),
        _ => None,
    }
}

fn notify_error(msg: &str) {
    let _ = synshell_common::ipc::send_action(synshell_common::Action::Shell(format!("screenrec-failed {msg}")));
}

pub fn main(args: &[String]) -> i32 {
    let (cfg, _) = Config::load();
    let mut o = Opts::from_config(&cfg.recording);
    let mut action = "toggle".to_string();
    let mut wait = false;
    let mut i = 0;
    let value = |i: usize| args.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "toggle" | "start" | "stop" | "status" | "wait" => action = a.to_string(),
            "--screen" | "--output" => {
                o.mode = Mode::Screen;
                if let Some(v) = value(i) {
                    o.output = Some(v);
                    i += 1;
                }
            }
            "--area" | "--region" => {
                o.mode = Mode::Area;
                if let Some(v) = value(i) {
                    let n: Vec<f64> = v.split([',', 'x', ' ']).filter_map(|x| x.trim().parse().ok()).collect();
                    if n.len() != 4 {
                        eprintln!("synshot: {}", t!("область задаётся как X,Y,Ш,В: {v}", v = v));
                        return 2;
                    }
                    o.region = Some([n[0], n[1], n[2], n[3]]);
                    i += 1;
                }
            }
            "--window" => {
                o.mode = Mode::Window;
                if let Some(v) = value(i) {
                    o.window = v.parse().ok();
                    i += 1;
                }
            }
            "--mode" => {
                if let Some(m) = value(i).and_then(|v| Mode::parse(&v)) {
                    o.mode = m;
                        i += 1;
                }
            }
            "--fps" => {
                if let Some(v) = value(i).and_then(|v| v.parse().ok()) {
                    o.fps = v;
                    i += 1;
                }
            }
            "--audio" | "--mic" | "--cursor" => {
                let v = value(i).and_then(|v| on(&v));
                if v.is_some() {
                    i += 1;
                }
                let v = v.unwrap_or(true);
                match a {
                    "--audio" => o.audio = v,
                    "--mic" => o.mic = v,
                    _ => o.cursor = v,
                }
            }
            "--no-audio" => o.audio = false,
            "--no-mic" => o.mic = false,
            "--no-cursor" => o.cursor = false,
            "--codec" | "--encoder" | "--quality" => {
                if let Some(v) = value(i) {
                    match a {
                        "--codec" => o.codec = v,
                        "--encoder" => o.encoder = v,
                        _ => o.quality = v,
                    }
                    i += 1;
                }
            }
            "--duration" => {
                if let Some(v) = value(i).and_then(|v| v.parse().ok()) {
                    o.duration = Some(v);
                    i += 1;
                }
            }
            "--out" | "-o" => {
                if let Some(v) = value(i) {
                    o.path = Some(synshell_common::paths::expand_tilde(&v));
                    i += 1;
                }
            }
            "--wait" => wait = true,
            "-h" | "--help" => {
                println!("{HELP}");
                return 0;
            }
            _ => {}
        }
        i += 1;
    }
    let r = match action.as_str() {
        "status" => {
            match screenrec::current() {
                Some(s) => print(&s),
                None => {
                    let last = screenrec::read();
                    println!("{}", serde_json::json!({"state": "idle", "last": last}));
                }
            }
            return 0;
        }
        "wait" => screenrec::wait(),
        "stop" => screenrec::stop(),
        "toggle" if screenrec::current().is_some() => screenrec::stop(),
        _ => match start(o, &cfg) {
            Ok(None) => return 1,
            Ok(Some(s)) if wait => {
                print(&s);
                screenrec::wait()
            }
            Ok(Some(s)) => Ok(s),
            Err(e) => {
                let msg = format!("{e:#}");
                if action == "toggle" {
                    notify_error(&msg);
                }
                Err(e)
            }
        },
    };
    match r {
        Ok(s) => {
            print(&s);
            if s.state == "error" {
                1
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("synshot: {e:#}");
            1
        }
    }
}

fn start(mut o: Opts, cfg: &Config) -> anyhow::Result<Option<Status>> {
    if o.mode == Mode::Area && o.region.is_none() {
        match pick_area()? {
            Some(r) => o.region = Some(r),
            None => return Ok(None),
        }
    }
    screenrec::start(&o, &cfg.recording).map(Some)
}

fn pick_area() -> anyhow::Result<Option<[f64; 4]>> {
    let shot = Shot::capture_now().and_then(Shot::load)?;
    let (cfg, _) = Config::load();
    let mss = crate::ui::stylesheet(&cfg);
    let font = Some(cfg.appearance.font.trim().to_string()).filter(|f| !f.is_empty());
    crate::sel::set_record_mode(true);
    syngui_layer::run(syngui_layer::RunOptions { font_family: font }, &mss, move || {
        crate::sel::init(shot, false);
        crate::ui::open_surfaces();
    })?;
    Ok(crate::sel::result().filter(|(c, _, _)| *c == crate::sel::Choice::Record).map(|(_, r, _)| [r.x, r.y, r.w, r.h]))
}
