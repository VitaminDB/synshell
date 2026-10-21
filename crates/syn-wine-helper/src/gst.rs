//! Модули GStreamer для видео и звука в Wine (winegstreamer): нет демультиплексора или декодера —
//! наш Wine (crates/syn-wine/winegstreamer-missing-plugin.patch) зовёт
//! `syn-wine-helper gst-missing --exe ИМЯ -- ЧТО…`, где ЧТО — `caps:<caps>` (нечем разобрать или
//! декодировать такой поток), `plugins:<набор>` (base, good, ugly, bad, libav) или `element:<имя>`.
//! Здесь — какие пакеты Arch это дают; окно предлагает поставить их через pacman (synpkg).

use std::collections::HashMap;

use syngui::t;

/// Пакет с модулями GStreamer.
#[derive(Debug, Clone)]
pub struct GstPkg {
    pub name: &'static str,
    pub about: fn() -> String,
}

impl PartialEq for GstPkg {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for GstPkg {}

fn about_good() -> String {
    t!("Основные форматы: AVI, MKV, MP4/MOV, FLAC, VP8/VP9")
}
fn about_ugly() -> String {
    t!("ASF/WMV-контейнер, MPEG-1/2, AC-3 — форматы с лицензионными ограничениями")
}
fn about_bad() -> String {
    t!("Новые и редкие форматы: AV1, MPEG-TS, HEVC и другие")
}
fn about_libav() -> String {
    t!("Кодеки FFmpeg: WMV, WMA, H.264, MPEG-4, AAC, Indeo, Cinepak и другие")
}
fn about_base() -> String {
    t!("Базовые модули: Ogg, Vorbis, Theora, Opus, преобразование видео и звука")
}

pub const GOOD: GstPkg = GstPkg { name: "gst-plugins-good", about: about_good };
pub const UGLY: GstPkg = GstPkg { name: "gst-plugins-ugly", about: about_ugly };
pub const BAD: GstPkg = GstPkg { name: "gst-plugins-bad", about: about_bad };
pub const LIBAV: GstPkg = GstPkg { name: "gst-libav", about: about_libav };
pub const BASE: GstPkg = GstPkg { name: "gst-plugins-base", about: about_base };

/// Медиатип caps → пакеты, в которых обычно есть разбор/декодер. Первое совпадение по префиксу.
const BY_MEDIA: &[(&str, &[GstPkg])] = &[
    ("video/x-ms-asf", &[UGLY, LIBAV]),
    ("video/x-wmv", &[LIBAV]),
    ("audio/x-wma", &[LIBAV]),
    ("video/x-h264", &[LIBAV]),
    ("video/x-h265", &[LIBAV, BAD]),
    ("video/x-msmpeg", &[LIBAV]),
    ("video/x-divx", &[LIBAV]),
    ("video/x-xvid", &[LIBAV]),
    ("video/x-indeo", &[LIBAV]),
    ("video/x-cinepak", &[LIBAV]),
    ("video/x-svq", &[LIBAV]),
    ("video/x-flash-video", &[GOOD, LIBAV]),
    ("video/mpeg", &[LIBAV, UGLY]),
    ("audio/mpeg", &[LIBAV, UGLY]),
    ("audio/x-ac3", &[UGLY, LIBAV]),
    ("audio/x-eac3", &[LIBAV]),
    ("audio/x-dts", &[LIBAV]),
    ("audio/x-adpcm", &[LIBAV]),
    ("audio/x-alac", &[LIBAV]),
    ("audio/x-qdm2", &[LIBAV]),
    ("audio/x-true-hd", &[LIBAV]),
    ("video/x-av1", &[BAD]),
    ("video/mpegts", &[BAD]),
    ("video/x-msvideo", &[GOOD]),
    ("video/quicktime", &[GOOD]),
    ("video/x-matroska", &[GOOD]),
    ("video/webm", &[GOOD]),
    ("video/x-vp8", &[GOOD]),
    ("video/x-vp9", &[GOOD]),
    ("audio/x-flac", &[GOOD]),
    ("audio/x-wav", &[GOOD]),
    ("audio/x-mulaw", &[GOOD]),
    ("audio/x-alaw", &[GOOD]),
    ("application/ogg", &[BASE]),
    ("audio/ogg", &[BASE]),
    ("audio/x-vorbis", &[BASE]),
    ("video/x-theora", &[BASE]),
    ("audio/x-opus", &[BASE]),
];

/// Не знаем, чем это открыть, — всё, где обычно находятся кодеки.
const FALLBACK: &[GstPkg] = &[GOOD, UGLY, LIBAV, BAD];

/// Что не нашлось (аргументы `gst-missing` после `--`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Missing {
    /// Имя процесса Windows (`Fable.exe`, из /proc/self/comm — до 15 символов).
    pub exe: String,
    pub what: Vec<String>,
}

impl Missing {
    pub fn parse(args: &mut dyn Iterator<Item = String>) -> Missing {
        let mut m = Missing::default();
        while let Some(a) = args.next() {
            match a.as_str() {
                "--exe" => m.exe = args.next().unwrap_or_default(),
                "--" => {
                    while let Some(w) = args.next() {
                        m.what.push(w);
                    }
                }
                _ => m.what.push(a),
            }
        }
        m.what.dedup();
        m
    }

    /// Медиатипы для показа: `video/x-ms-asf`, `audio/x-wma` (без параметров caps).
    pub fn formats(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for w in &self.what {
            let f = match w.split_once(':') {
                Some(("caps", caps)) => caps.split([',', ';']).next().unwrap_or(caps).trim().to_string(),
                Some(("element", e)) => e.to_string(),
                _ => continue,
            };
            if !f.is_empty() && !v.contains(&f) {
                v.push(f);
            }
        }
        v
    }

    /// Пакеты, которые могут помочь (ещё без учёта установленных).
    pub fn candidates(&self) -> Vec<GstPkg> {
        let mut v: Vec<GstPkg> = Vec::new();
        let mut add = |p: &GstPkg| {
            if !v.contains(p) {
                v.push(p.clone());
            }
        };
        for w in &self.what {
            match w.split_once(':') {
                Some(("caps", caps)) => {
                    let media = caps.split([',', ';']).next().unwrap_or(caps).trim();
                    match BY_MEDIA.iter().find(|(m, _)| media.starts_with(m)) {
                        Some((_, pkgs)) => pkgs.iter().for_each(&mut add),
                        None => FALLBACK.iter().for_each(&mut add),
                    }
                }
                Some(("plugins", set)) => match set {
                    "base" => add(&BASE),
                    "good" => add(&GOOD),
                    "ugly" => add(&UGLY),
                    "bad" => add(&BAD),
                    "libav" => add(&LIBAV),
                    _ => FALLBACK.iter().for_each(&mut add),
                },
                _ => FALLBACK.iter().for_each(&mut add),
            }
        }
        v
    }

    /// Пакеты к установке: кандидаты, которых нет в системе.
    pub fn to_install(&self, installed: &HashMap<String, String>) -> Vec<GstPkg> {
        self.candidates().into_iter().filter(|p| !installed.contains_key(p.name)).collect()
    }
}

/// Один вопрос про эти пакеты за сеанс: Wine зовёт помощника из каждого процесса и на каждый поток,
/// а отказ не должен всплывать снова до перезахода. `false` — уже спрашивали.
pub fn claim(pkgs: &[GstPkg]) -> bool {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("syn-wine-helper");
    let _ = std::fs::create_dir_all(&dir);
    let mut names: Vec<&str> = pkgs.iter().map(|p| p.name).collect();
    names.sort();
    let mark = dir.join(format!("gst-{}", names.join("+")));
    std::fs::OpenOptions::new().write(true).create_new(true).open(mark).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(what: &[&str]) -> Missing {
        Missing { exe: "Fable.exe".into(), what: what.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn parse_args() {
        let a = ["--exe", "Fable.exe", "--", "caps:video/x-ms-asf", "plugins:ugly"];
        let got = Missing::parse(&mut a.iter().map(|s| s.to_string()));
        assert_eq!(got, m(&["caps:video/x-ms-asf", "plugins:ugly"]));
    }

    #[test]
    fn asf_needs_ugly_then_libav() {
        let names: Vec<_> = m(&["caps:video/x-ms-asf"]).candidates().iter().map(|p| p.name).collect();
        assert_eq!(names, ["gst-plugins-ugly", "gst-libav"]);
    }

    #[test]
    fn wmv_caps_with_params() {
        let x = m(&["caps:video/x-wmv, wmvversion=(int)3, width=(int)640", "caps:audio/x-wma, wmaversion=(int)2"]);
        assert_eq!(x.formats(), ["video/x-wmv", "audio/x-wma"]);
        assert_eq!(x.candidates().iter().map(|p| p.name).collect::<Vec<_>>(), ["gst-libav"]);
    }

    #[test]
    fn installed_filtered_unknown_falls_back() {
        let mut inst = HashMap::new();
        inst.insert("gst-plugins-good".to_string(), "1".to_string());
        inst.insert("gst-libav".to_string(), "1".to_string());
        let names: Vec<_> = m(&["caps:video/x-strange"]).to_install(&inst).iter().map(|p| p.name).collect();
        assert_eq!(names, ["gst-plugins-ugly", "gst-plugins-bad"]);
    }

    #[test]
    fn plugin_set() {
        assert_eq!(m(&["plugins:libav"]).candidates().iter().map(|p| p.name).collect::<Vec<_>>(), ["gst-libav"]);
    }
}
