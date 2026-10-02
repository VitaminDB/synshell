//! Настройки «Камеры» — `~/.config/syncamera.conf` (ключ = значение): переживают перезапуск программы.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn path() -> PathBuf {
    let cfg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config"));
    cfg.join("syncamera.conf")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub flash: u32,
    pub timer: u32,
    pub grid: bool,
    /// 0 — 4:3, 1 — 16:9, 2 — 1:1
    pub aspect: u32,
    pub full_res: bool,
    /// 0 — 720p, 1 — 1080p, 2 — 4K
    pub video_q: u32,
    pub hevc: bool,
    pub mic: bool,
    pub stab: bool,
    pub sound: bool,
    pub timelapse: u32,
    pub front: bool,
    pub mode: u32,
    /// Задний объектив последнего сеанса: 0 — основная, 1 — широкоугольная, 2 — макро.
    pub lens: u32,
    /// Камера при запуске: 0 — как в прошлый раз, 1 — основная, 2 — широкоугольная, 3 — фронтальная.
    pub start_cam: u32,
    /// Папки снимков и видео; пусто — «Изображения» и «Видео» пользователя (XDG).
    pub photo_dir: String,
    pub video_dir: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            flash: 0,
            timer: 0,
            grid: false,
            aspect: 0,
            full_res: false,
            video_q: 1,
            hevc: false,
            mic: true,
            stab: true,
            sound: true,
            timelapse: 10,
            front: false,
            mode: 2,
            lens: 0,
            start_cam: 1,
            photo_dir: String::new(),
            video_dir: String::new(),
        }
    }
}

impl Prefs {
    pub fn load() -> Prefs {
        let mut p = Prefs::default();
        let Ok(s) = std::fs::read_to_string(path()) else { return p };
        let kv: BTreeMap<&str, &str> = s.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect();
        let num = |k: &str, d: u32| kv.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        let flag = |k: &str, d: bool| kv.get(k).map(|v| *v == "1" || *v == "true").unwrap_or(d);
        p.flash = num("flash", p.flash).min(2);
        p.timer = num("timer", p.timer);
        p.grid = flag("grid", p.grid);
        p.aspect = num("aspect", p.aspect).min(2);
        p.full_res = flag("full_res", p.full_res);
        p.video_q = num("video_quality", p.video_q).min(2);
        p.hevc = flag("hevc", p.hevc);
        p.mic = flag("mic", p.mic);
        p.stab = flag("stabilization", p.stab);
        p.sound = flag("sound", p.sound);
        p.timelapse = num("timelapse", p.timelapse).clamp(2, 120);
        p.front = flag("front", p.front);
        p.mode = num("mode", p.mode).min(4);
        p.lens = num("lens", p.lens).min(2);
        p.start_cam = num("start_camera", p.start_cam).min(3);
        p.photo_dir = kv.get("photo_dir").map(|s| s.to_string()).unwrap_or_default();
        p.video_dir = kv.get("video_dir").map(|s| s.to_string()).unwrap_or_default();
        p
    }

    pub fn save(&self) {
        let b = |v: bool| if v { "1" } else { "0" };
        let s = format!(
            "# «Камера» synshell\nflash = {}\ntimer = {}\ngrid = {}\naspect = {}\nfull_res = {}\nvideo_quality = {}\nhevc = {}\nmic = {}\nstabilization = {}\nsound = {}\ntimelapse = {}\nfront = {}\nmode = {}\nlens = {}\nstart_camera = {}\nphoto_dir = {}\nvideo_dir = {}\n",
            self.flash,
            self.timer,
            b(self.grid),
            self.aspect,
            b(self.full_res),
            self.video_q,
            b(self.hevc),
            b(self.mic),
            b(self.stab),
            b(self.sound),
            self.timelapse,
            b(self.front),
            self.mode,
            self.lens,
            self.start_cam,
            self.photo_dir,
            self.video_dir,
        );
        let p = path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, s);
    }
}
