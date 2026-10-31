//! Субтитры: кнопка в шапке плеера и панель — дорожка, синхронизация, оформление (готовые наборы,
//! шрифт, размер, цвета, контур, тень, плашка, положение, анимации) с живым образцом; настройки
//! (`~/.config/synshell/video-player-subtitles.toml`); выбор дорожки при открытии ролика и клавиши
//! (C — вкл/выкл, Z/X — субтитры раньше/позже на 0,1 с).
//!
//! Внешние файлы рядом с роликом (`Фильм.srt`, `Фильм.rus.ass`, `Subs/…`) подключаются сами.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use syngui::prelude::*;
use syngui::video::subtitle::render::render_sample;
use syngui::video::{SubtitleSource, SubtitleStyle, SubtitleTrackInfo, VideoPlayer};
use syngui::video::subtitle::SubtitleAnimation;
use syngui::widgets::*;

use crate::ui::{icons, St};

type W = Box<dyn Widget>;

// ─── настройки ──────────────────────────────────────────────────────────────

/// Когда включать субтитры сами.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Auto {
    /// Только вручную.
    Off,
    /// Озвучка на другом языке (а на своём — только надписи, «forced»).
    Foreign,
    /// Всегда, если есть дорожка на нужном языке.
    Always,
}

/// Оформление и поведение субтитров. Доли — в процентах (как в интерфейсе).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubSettings {
    pub auto: Auto,
    /// Язык субтитров (ISO 639-1).
    pub language: String,
    pub font: String,
    pub weight: u16,
    /// Кегль, % высоты кадра.
    pub size: f32,
    pub color: String,
    /// Непрозрачность текста, %.
    pub opacity: f32,
    /// Контур, % кегля.
    pub outline: f32,
    pub outline_color: String,
    /// Мягкость контура, % кегля.
    pub softness: f32,
    /// Тень: непрозрачность, сдвиг и размытие (% кегля).
    pub shadow: f32,
    pub shadow_offset: f32,
    pub shadow_blur: f32,
    pub shadow_color: String,
    /// Плашка: непрозрачность, %.
    pub box_opacity: f32,
    pub box_color: String,
    pub line_spacing: f32,
    /// Отступ от края, % высоты.
    pub margin: f32,
    /// Ширина строки, % ширины кадра.
    pub max_width: f32,
    pub in_frame: bool,
    pub anim_in: String,
    pub anim_in_ms: u32,
    pub anim_out: String,
    pub anim_out_ms: u32,
    pub file_colors: bool,
    pub file_positions: bool,
}

impl Default for SubSettings {
    fn default() -> Self {
        let mut s = Self {
            auto: Auto::Foreign,
            language: system_language(),
            font: String::new(),
            weight: 0,
            size: 0.0,
            color: String::new(),
            opacity: 100.0,
            outline: 0.0,
            outline_color: String::new(),
            softness: 0.0,
            shadow: 0.0,
            shadow_offset: 0.0,
            shadow_blur: 0.0,
            shadow_color: String::new(),
            box_opacity: 0.0,
            box_color: String::new(),
            line_spacing: 1.12,
            margin: 5.5,
            max_width: 86.0,
            in_frame: true,
            anim_in: "rise".into(),
            anim_in_ms: 180,
            anim_out: "fade".into(),
            anim_out_ms: 160,
            file_colors: true,
            file_positions: true,
        };
        PRESETS[0].apply(&mut s);
        s
    }
}

/// Язык системы (`LANG=ru_RU.UTF-8` → «ru»).
fn system_language() -> String {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            let code: String = v.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_lowercase();
            if code.len() == 2 {
                return code;
            }
        }
    }
    "ru".into()
}

fn hex(s: &str) -> [u8; 3] {
    let h = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0xFFFFFF);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn anim(s: &str) -> SubtitleAnimation {
    match s {
        "none" => SubtitleAnimation::None,
        "fade" => SubtitleAnimation::Fade,
        "zoom" => SubtitleAnimation::Zoom,
        _ => SubtitleAnimation::Rise,
    }
}

impl SubSettings {
    pub fn style(&self) -> SubtitleStyle {
        SubtitleStyle {
            font_family: self.font.clone(),
            font_weight: self.weight,
            size: self.size / 100.0,
            min_size_px: 15.0,
            color: hex(&self.color),
            opacity: self.opacity / 100.0,
            outline: self.outline / 100.0,
            outline_color: hex(&self.outline_color),
            outline_softness: self.softness / 100.0,
            shadow: self.shadow_offset / 100.0,
            shadow_blur: self.shadow_blur / 100.0,
            shadow_color: hex(&self.shadow_color),
            shadow_opacity: self.shadow / 100.0,
            box_color: hex(&self.box_color),
            box_opacity: self.box_opacity / 100.0,
            line_spacing: self.line_spacing,
            letter_spacing: 0.0,
            max_width: self.max_width / 100.0,
            margin: self.margin / 100.0,
            in_frame: self.in_frame,
            anim_in: anim(&self.anim_in),
            anim_in_ms: self.anim_in_ms,
            anim_out: anim(&self.anim_out),
            anim_out_ms: self.anim_out_ms,
            file_colors: self.file_colors,
            file_positions: self.file_positions,
            bitmap_opacity: 1.0,
        }
    }

    pub fn load() -> Self {
        std::fs::read_to_string(file()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let f = file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            if let Err(e) = std::fs::write(&f, s) {
                tracing::warn!("{}: {e}", f.display());
            }
        }
    }
}

fn file() -> PathBuf {
    synshell_common::paths::config_dir().join("video-player-subtitles.toml")
}

/// Готовый набор оформления.
pub struct Preset {
    pub name: &'static str,
    font: &'static str,
    weight: u16,
    size: f32,
    color: &'static str,
    outline: f32,
    outline_color: &'static str,
    softness: f32,
    shadow: f32,
    shadow_offset: f32,
    shadow_blur: f32,
    box_opacity: f32,
}

impl Preset {
    fn apply(&self, s: &mut SubSettings) {
        s.font = self.font.into();
        s.weight = self.weight;
        s.size = self.size;
        s.color = self.color.into();
        s.opacity = 100.0;
        s.outline = self.outline;
        s.outline_color = self.outline_color.into();
        s.softness = self.softness;
        s.shadow = self.shadow;
        s.shadow_offset = self.shadow_offset;
        s.shadow_blur = self.shadow_blur;
        s.shadow_color = "#000000".into();
        s.box_opacity = self.box_opacity;
        s.box_color = "#000000".into();
    }

    fn matches(&self, s: &SubSettings) -> bool {
        let mut t = s.clone();
        self.apply(&mut t);
        t == *s
    }
}

pub const PRESETS: &[Preset] = &[
    Preset { name: n_!("Чистый"), font: "Golos Text", weight: 600, size: 5.5, color: "#ffffff", outline: 7.0, outline_color: "#000000", softness: 1.5, shadow: 55.0, shadow_offset: 5.0, shadow_blur: 8.0, box_opacity: 0.0 },
    Preset { name: n_!("Плашка"), font: "Manrope", weight: 600, size: 5.0, color: "#ffffff", outline: 0.0, outline_color: "#000000", softness: 0.0, shadow: 0.0, shadow_offset: 0.0, shadow_blur: 0.0, box_opacity: 72.0 },
    Preset { name: n_!("Мягкий"), font: "Nunito", weight: 700, size: 5.6, color: "#fffdf5", outline: 4.0, outline_color: "#1a1a1a", softness: 4.0, shadow: 75.0, shadow_offset: 3.0, shadow_blur: 22.0, box_opacity: 0.0 },
    Preset { name: n_!("Кино"), font: "Rubik", weight: 500, size: 5.4, color: "#ffe680", outline: 6.0, outline_color: "#000000", softness: 1.0, shadow: 50.0, shadow_offset: 5.0, shadow_blur: 6.0, box_opacity: 0.0 },
    Preset { name: n_!("Тонкий"), font: "Onest", weight: 500, size: 5.2, color: "#ffffff", outline: 2.5, outline_color: "#000000", softness: 2.0, shadow: 85.0, shadow_offset: 2.0, shadow_blur: 14.0, box_opacity: 0.0 },
    Preset { name: n_!("Крупный"), font: "Golos Text", weight: 700, size: 7.6, color: "#ffffff", outline: 9.0, outline_color: "#000000", softness: 1.0, shadow: 60.0, shadow_offset: 5.0, shadow_blur: 8.0, box_opacity: 0.0 },
];

/// Шрифты в комплекте (`/usr/share/fonts/synshell`): название и чем хорош.
const FONTS: &[(&str, &str)] = &[
    ("Golos Text", n_!("спокойный, читается с первого взгляда")),
    ("Manrope", n_!("современный и чёткий")),
    ("Onest", n_!("мягкий гротеск")),
    ("Nunito", n_!("округлый и дружелюбный")),
    ("Rubik", n_!("плотный, со скруглёнными углами")),
    ("Jost", n_!("геометрический")),
    ("Comfortaa", n_!("округлый и лёгкий")),
    ("Inter", n_!("нейтральный, как в интерфейсе")),
];

const COLORS: &[&str] = &["#ffffff", "#fffdf0", "#ffe680", "#ffd23f", "#9ef0ff", "#b8ffb0", "#ffc4e1"];
const OUTLINE_COLORS: &[&str] = &["#000000", "#1a1a1a", "#3a3a3a", "#0b1e3d", "#3d0b0b", "#ffffff"];
const BOX_COLORS: &[&str] = &["#000000", "#1e1e1e", "#202838", "#ffffff"];

/// Языки на выбор: код и название.
const LANGUAGES: &[&str] = &["ru", "en", "uk", "be", "kk", "de", "fr", "es", "it", "pt", "pl", "ja", "ko", "zh", "tr"];

pub fn language_name(code: &str) -> String {
    let n = match syngui::video::subtitle::language_code_normalized(code).as_str() {
        "ru" => t!("Русский"),
        "en" => t!("Английский"),
        "uk" => t!("Украинский"),
        "be" => t!("Белорусский"),
        "kk" => t!("Казахский"),
        "de" => t!("Немецкий"),
        "fr" => t!("Французский"),
        "es" => t!("Испанский"),
        "it" => t!("Итальянский"),
        "pt" => t!("Португальский"),
        "pl" => t!("Польский"),
        "ja" => t!("Японский"),
        "ko" => t!("Корейский"),
        "zh" => t!("Китайский"),
        "tr" => t!("Турецкий"),
        _ => return code.to_string(),
    };
    n.to_string()
}

fn same_lang(a: Option<&str>, b: &str) -> bool {
    a.is_some_and(|a| syngui::video::subtitle::language_code_normalized(a) == syngui::video::subtitle::language_code_normalized(b))
}

// ─── выбор дорожки ──────────────────────────────────────────────────────────

/// Что пользователь выбрал в этом сеансе — следующие ролики откроются так же.
#[derive(Clone, Debug, PartialEq)]
pub enum SubPref {
    Off,
    Track { title: Option<String>, language: Option<String>, forced: bool },
}

impl SubPref {
    fn of(t: &SubtitleTrackInfo) -> Self {
        Self::Track { title: t.title.clone(), language: t.language.clone(), forced: t.forced }
    }
}

/// Лучшая из подходящих: обычная (не надписи, не SDH), помеченная основной, текстовая, своя
/// (внешний файл положили рядом намеренно).
fn best<'a>(tracks: impl Iterator<Item = &'a SubtitleTrackInfo>) -> Option<&'a SubtitleTrackInfo> {
    tracks.max_by_key(|t| (!t.forced, !t.hearing_impaired, t.default, !t.bitmap, t.is_external()))
}

/// Дорожка при открытии ролика: выбор сеанса, иначе — по настройке «Включать автоматически».
pub fn pick(tracks: &[SubtitleTrackInfo], audio_lang: Option<&str>, s: &SubSettings, pref: Option<&SubPref>) -> Option<SubtitleSource> {
    match pref {
        Some(SubPref::Off) => return None,
        Some(SubPref::Track { title, language, forced }) => {
            let by_title = title.as_ref().and_then(|ti| {
                tracks.iter().find(|t| t.title.as_ref().is_some_and(|x| x.trim().eq_ignore_ascii_case(ti.trim())) && (language.is_none() || t.language == *language))
            });
            let by_lang = || best(tracks.iter().filter(|t| t.forced == *forced && language.as_deref().is_some_and(|l| same_lang(t.language.as_deref(), l))));
            if let Some(t) = by_title.or_else(by_lang) {
                return Some(t.source.clone());
            }
        }
        None => {}
    }
    let lang = s.language.as_str();
    let in_lang = |t: &&SubtitleTrackInfo| same_lang(t.language.as_deref(), lang);
    match s.auto {
        Auto::Off => None,
        Auto::Always => best(tracks.iter().filter(in_lang))
            .or_else(|| best(tracks.iter().filter(|t| t.is_external() && t.language.is_none())))
            .map(|t| t.source.clone()),
        Auto::Foreign => {
            if same_lang(audio_lang, lang) {
                // своя озвучка — только надписи на своём языке
                return best(tracks.iter().filter(|t| t.forced && in_lang(t))).map(|t| t.source.clone());
            }
            if audio_lang.is_some() {
                return best(tracks.iter().filter(|t| !t.forced).filter(in_lang))
                    .or_else(|| best(tracks.iter().filter(|t| t.is_external() && t.language.is_none())))
                    .map(|t| t.source.clone());
            }
            // Язык озвучки не указан: внешний файл положили рядом — показать; из дорожек файла —
            // только помеченную основной.
            best(tracks.iter().filter(|t| t.is_external() && (t.language.is_none() || in_lang(t))))
                .or_else(|| best(tracks.iter().filter(|t| t.default && !t.forced).filter(in_lang)))
                .map(|t| t.source.clone())
        }
    }
}

/// Только что открытый ролик (поток открытия): подключить файлы рядом, выбрать дорожку.
pub fn on_open(st: St, path: &Path, p: &mut VideoPlayer) {
    let store = p.subtitles();
    if path.is_file() {
        let n = store.add_external_near(path);
        if n > 0 {
            tracing::info!("субтитры: подключено файлов рядом с роликом — {n}");
        }
    }
    let tracks = store.tracks();
    let audio = p.audio_track().and_then(|i| p.audio_tracks().iter().find(|t| t.stream_index == i)).and_then(|t| t.language.clone());
    let choice = pick(&tracks, audio.as_deref(), &st.subs.get_untracked(), st.sub_pref.get_untracked().as_ref());
    if let Some(src) = &choice {
        tracing::info!("субтитры: дорожка {src:?}");
    }
    p.set_subtitle_track(choice.as_ref());
}

/// Сменили озвучку: субтитры, которые в этом сеансе вручную не выбирали, — под её язык
/// (английская озвучка — русские субтитры, русская — только надписи).
pub fn on_audio_changed(st: St, audio_lang: Option<&str>) {
    if st.sub_pref.get_untracked().is_some() {
        return;
    }
    let Some(p) = st.playing.get_untracked() else { return };
    let Ok(mut pl) = p.player.lock() else { return };
    let tracks = pl.subtitles().tracks();
    let choice = pick(&tracks, audio_lang, &st.subs.get_untracked(), None);
    pl.set_subtitle_track(choice.as_ref());
    drop(pl);
    st.sub_rev.update(|v| *v += 1);
}

fn store_of(st: St) -> Option<std::sync::Arc<syngui::video::SubtitleStore>> {
    let p = st.playing.get_untracked()?;
    let pl = p.player.lock().ok()?;
    Some(pl.subtitles())
}

fn select(st: St, track: Option<&SubtitleTrackInfo>) {
    let Some(p) = st.playing.get_untracked() else { return };
    if let Ok(mut pl) = p.player.lock() {
        pl.set_subtitle_track(track.map(|t| &t.source));
    }
    if let Some(t) = track {
        st.sub_last.set(Some(t.source.clone()));
    }
    st.sub_pref.set(Some(track.map(SubPref::of).unwrap_or(SubPref::Off)));
    st.sub_rev.update(|v| *v += 1);
}

/// C: выключить или вернуть последнюю (или подходящую) дорожку.
pub fn toggle(st: St) {
    let Some(store) = store_of(st) else { return };
    let tracks = store.tracks();
    if store.selected().is_some() {
        select(st, None);
        return;
    }
    let last = st.sub_last.get_untracked();
    let t = last
        .and_then(|l| tracks.iter().find(|t| t.source == l))
        .or_else(|| {
            let lang = st.subs.get_untracked().language;
            best(tracks.iter().filter(|t| same_lang(t.language.as_deref(), &lang))).or_else(|| best(tracks.iter()))
        })
        .cloned();
    if let Some(t) = t {
        select(st, Some(&t));
    }
}

fn shift(st: St, by: f64) {
    if let Some(store) = store_of(st) {
        store.set_delay(store.delay() + by);
        st.sub_rev.update(|v| *v += 1);
    }
}

/// Клавиши плеера: C — вкл/выкл, Z/X — субтитры раньше/позже (с Shift — на 0,5 с).
pub fn on_key(st: St, key: Key, mods: Modifiers) -> bool {
    if mods.ctrl || mods.alt || mods.meta {
        return false;
    }
    let step = if mods.shift { 0.5 } else { 0.1 };
    match key {
        Key::C => toggle(st),
        Key::Z => shift(st, -step),
        Key::X => shift(st, step),
        _ => return false,
    }
    true
}

pub const KEYS: [Key; 3] = [Key::C, Key::Z, Key::X];

// ─── шапка и панель ─────────────────────────────────────────────────────────

/// Кнопка в шапке: приглушена, пока субтитры выключены.
pub fn button(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        st.sub_rev.get();
        let on = store_of(st).is_some_and(|s| s.selected().is_some());
        vec![Box::new(
            ToolButton::new(icons::SUBTITLES)
                .tooltip(t!("Субтитры (C)"))
                .on_click(move || st.subs_open.set(true))
                .class(if on { "vp-btn" } else { "vp-btn sub-btn-off" }),
        )]
    }))
}

fn update(st: St, f: impl FnOnce(&mut SubSettings)) {
    let mut s = st.subs.get_untracked();
    f(&mut s);
    if s != st.subs.get_untracked() {
        st.sub_style.set(s.style());
        s.save();
        st.subs.set(s);
    }
}

/// Целиком перестроить оформление (набор, сброс): ползунки берут значения при построении.
fn update_all(st: St, f: impl FnOnce(&mut SubSettings)) {
    update(st, f);
    st.sub_gen.update(|v| *v += 1);
}

pub fn panel(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        if !st.subs_open.get() {
            return vec![];
        }
        let close = move || st.subs_open.set(false);
        let body = Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(tracks_card(st))
            .child(sync_card(st))
            .child(Reactive::new(move || -> Vec<W> {
                st.sub_gen.get();
                vec![look_card(st)]
            }))
            .child(auto_card(st))
            .class("set-body");
        let panel = Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(t!("Субтитры")).class("set-title"))
                    .child(DecoratedBox::new().class("grow"))
                    .child(ToolButton::new(icons::CLOSE).tooltip(t!("Закрыть")).on_click(close).class("set-close"))
                    .class("set-head"),
            )
            .child(ScrollView::new().vertical().child(body).class("grow"))
            .class("set-panel sub-panel");
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(close).child(DecoratedBox::new().class("sub-scrim")))
                .child(
                    Row::new()
                        .main_axis_alignment(MainAxisAlignment::End)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(GestureDetector::new().on_click(|| {}).child(panel)),
                ),
        )]
    }))
}

fn card(icon: &'static str, title: &str, rows: Vec<W>) -> W {
    let mut c = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(icon).class("set-sec-icon"))
            .child(Text::new(title).class("set-sec-title")),
    );
    for r in rows {
        c = c.child(r);
    }
    Box::new(DecoratedBox::new().child(c).class("set-card"))
}

fn hint(text: &str) -> W {
    Box::new(Text::new(text).max_lines(5).class("set-hint"))
}

fn label(text: &str) -> W {
    Box::new(Text::new(text).class("set-row-title"))
}

/// «Русский · SRT · внешний файл»
fn track_labels(t: &SubtitleTrackInfo, n: usize) -> (String, String) {
    let lang = t.language.as_deref().map(language_name);
    let title = t.title.clone().or_else(|| lang.clone()).unwrap_or_else(|| t!("Дорожка {n}", n = n).to_string());
    let mut sub: Vec<String> = Vec::new();
    if let Some(l) = lang.filter(|l| *l != title) {
        sub.push(l);
    }
    sub.push(t.codec.clone());
    if t.forced {
        sub.push(t!("только надписи").into());
    }
    if t.hearing_impaired {
        sub.push(t!("для слабослышащих").into());
    }
    if let SubtitleSource::External(p) = &t.source {
        sub.push(p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default());
    }
    if t.bitmap {
        sub.push(t!("картинки — оформление не меняется").into());
    }
    (title, sub.join(" · "))
}

fn track_row(title: String, sub: String, active: bool, on: impl Fn() + Send + Sync + 'static) -> W {
    let mut col = Column::new().gap(2.0).child(Text::new(title).max_lines(2).class("set-row-title"));
    if !sub.is_empty() {
        col = col.child(Text::new(sub).max_lines(3).class("set-hint"));
    }
    Box::new(
        GestureDetector::new().on_click(on).cursor(syngui::CursorIcon::Pointer).child(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(if active { icons::RADIO_ON } else { icons::RADIO_OFF }).class("track-radio"))
                        .child(col.class("grow")),
                )
                .class(if active { "track-row track-row-active" } else { "track-row" }),
        ),
    )
}

fn tracks_card(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        st.sub_rev.get();
        let Some(store) = store_of(st) else { return vec![] };
        let tracks = store.tracks();
        let current = store.selected();
        let mut rows: Vec<W> = vec![track_row(t!("Выключены").into(), String::new(), current.is_none(), move || select(st, None))];
        for (i, t) in tracks.iter().enumerate() {
            let (title, sub) = track_labels(t, i + 1);
            let tr = t.clone();
            rows.push(track_row(title, sub, current.as_ref() == Some(&t.source), move || select(st, Some(&tr))));
        }
        if tracks.is_empty() {
            rows.push(hint(&t!("В ролике нет субтитров. Положите рядом файл .srt, .ass или .vtt с тем же именем (или в папку Subs) — он подключится сам.")));
        }
        vec![card(icons::SUBTITLES, &t!("Дорожка"), rows)]
    }))
}

fn sync_card(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        st.sub_rev.get();
        let Some(store) = store_of(st) else { return vec![] };
        if store.selected().is_none() {
            return vec![];
        }
        let d = store.delay();
        let value = if d.abs() < 0.0005 { t!("вовремя").to_string() } else { format!("{:+.1} с", d).replace('.', ",") };
        let b = |label: &str, by: f64| -> W { Box::new(Button::new(label).on_click(move || shift(st, by)).class("sub-step")) };
        let row = Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(b("−0,5", -0.5))
            .child(b("−0,1", -0.1))
            .child(DecoratedBox::new().child(Text::new(value).class("sub-delay")).class("grow sub-delay-box"))
            .child(b("+0,1", 0.1))
            .child(b("+0,5", 0.5));
        let mut rows: Vec<W> = vec![Box::new(row)];
        if d.abs() >= 0.0005 {
            rows.push(Box::new(Button::new(t!("Сбросить")).icon(icons::UNDO).on_click(move || {
                if let Some(s) = store_of(st) {
                    s.set_delay(0.0);
                    st.sub_rev.update(|v| *v += 1);
                }
            })));
        }
        rows.push(hint(&t!("Плюс — субтитры позже, минус — раньше. Клавиши Z и X (с Shift — по 0,5 с).")));
        vec![card(icons::SYNC, &t!("Синхронизация"), rows)]
    }))
}

// ─── оформление ─────────────────────────────────────────────────────────────

/// Образец поверх «кадра»: как субтитры будут выглядеть.
fn preview(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let s = st.subs.get();
        let style = s.style();
        let scale = syngui::signal::primary_window().map(|w| w.scale_factor() as f32).unwrap_or(1.0).clamp(1.0, 4.0);
        // образец — как в окне ~720 px высотой
        let font = (style.size * 720.0).max(15.0) * 0.62;
        let text = format!("{}\n{{\\i1}}{}{{\\i0}}", t!("Субтитры будут выглядеть так."), t!("А это — курсив."));
        let img: W = match render_sample(&text, &style, font * scale, 320.0 * scale) {
            Some(r) => {
                let key = format!("sub-preview:{:x}:{}x{}", style.raster_key(), r.w, r.h);
                Box::new(
                    Image::from_rgba(key, r.w, r.h, r.rgba.to_vec())
                        .style("width", r.w as f32 / scale)
                        .style("height", r.h as f32 / scale),
                )
            }
            None => Box::new(Text::new(t!("Шрифт «{f}» не найден", f = s.font)).class("set-hint")),
        };
        vec![Box::new(
            DecoratedBox::new()
                .child(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).child(img).class("grow"))
                .class("sub-preview"),
        )]
    }))
}

fn presets(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let s = st.subs.get();
        let chips: Vec<W> = PRESETS
            .iter()
            .map(|p| -> W {
                let on = p.matches(&s);
                Box::new(
                    Button::new(t!(p.name))
                        .on_click(move || update_all(st, |s| p.apply(s)))
                        .class(if on { "sub-chip sub-chip-on" } else { "sub-chip" }),
                )
            })
            .collect();
        vec![Box::new(Flex::new().wrap().gap(6.0).children(chips))]
    }))
}

fn fonts(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let s = st.subs.get();
        let mut rows: Vec<W> = Vec::new();
        let installed = system_families();
        for (name, about) in FONTS.iter().filter(|(n, _)| installed.iter().any(|f| f == n)) {
            let on = s.font == *name;
            rows.push(Box::new(
                GestureDetector::new()
                    .on_click(move || update(st, |s| s.font = name.to_string()))
                    .cursor(syngui::CursorIcon::Pointer)
                    .child(
                        DecoratedBox::new()
                            .child(
                                Row::new()
                                    .gap(10.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Text::new(*name).style("font-family", syngui::StyleValue::String(name.to_string())).class("sub-font-name"))
                                    .child(Text::new(t!(*about)).max_lines(1).class("set-hint grow")),
                            )
                            .class(if on { "sub-font sub-font-on" } else { "sub-font" }),
                    ),
            ));
        }
        let custom = !FONTS.iter().any(|(n, _)| *n == s.font);
        let families = system_families();
        let items: Vec<DropdownItem> = families.iter().map(|f| DropdownItem::new(f.clone(), f.clone())).collect();
        let mut dd = Dropdown::with_items(items).placeholder(t!("Другой шрифт системы…")).max_height(320.0).on_change(move |v| {
            let v = v.to_string();
            update(st, |s| s.font = v)
        });
        if custom {
            dd = dd.selected(s.font.clone());
        }
        rows.push(Box::new(dd));
        vec![Box::new(Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(rows))]
    }))
}

fn system_families() -> &'static Vec<String> {
    static F: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    F.get_or_init(syngui::text::font_discovery::list_families)
}

/// Ряд образцов цвета.
fn swatches(st: St, colors: &'static [&'static str], get: fn(&SubSettings) -> &String, set: fn(&mut SubSettings, &str)) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let s = st.subs.get();
        let cur = get(&s).to_ascii_lowercase();
        let items: Vec<W> = colors
            .iter()
            .map(|c| -> W {
                let on = cur == *c;
                Box::new(
                    GestureDetector::new()
                        .on_click(move || update(st, |s| set(s, c)))
                        .cursor(syngui::CursorIcon::Pointer)
                        .child(
                            DecoratedBox::new()
                                .child(DecoratedBox::new().style("background-color", syngui::Color::from_hex(c)).class("sub-swatch-dot"))
                                .class(if on { "sub-swatch sub-swatch-on" } else { "sub-swatch" }),
                        ),
                )
            })
            .collect();
        vec![Box::new(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).children(items))]
    }))
}

/// Ползунок с подписью и значением справа.
fn slider(st: St, title: &str, v: f32, min: f32, max: f32, step: f32, fmt: fn(f32) -> String, get: fn(&SubSettings) -> f32, set: fn(&mut SubSettings, f32)) -> W {
    let value = Reactive::new(move || -> Vec<W> { vec![Box::new(Text::new(fmt(get(&st.subs.get()))).class("sub-value"))] });
    Box::new(
        Column::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(title).class("set-row-title grow")).child(value))
            .child(Slider::new().value(v).range(min, max).step(step).on_change(move |x| update(st, |s| set(s, x))).class("sub-slider")),
    )
}

fn pct(v: f32) -> String {
    format!("{v:.0} %")
}

fn pct1(v: f32) -> String {
    format!("{v:.1} %").replace('.', ",")
}

fn ms(v: f32) -> String {
    t!("{v} мс", v = format!("{v:.0}")).to_string()
}

fn times(v: f32) -> String {
    format!("×{v:.2}").replace('.', ",")
}

const WEIGHTS: [u16; 4] = [400, 500, 600, 800];
const ANIMS: [&str; 4] = ["none", "fade", "rise", "zoom"];

fn look_card(st: St) -> W {
    let s = st.subs.get_untracked();
    let anim_segs = |out: bool| -> Vec<String> {
        vec![
            t!("Нет").into(),
            t!("Растворение").into(),
            if out { t!("Подъём").into() } else { t!("Всплытие").into() },
            t!("Масштаб").into(),
        ]
    };
    let rows: Vec<W> = vec![
        preview(st),
        label(&t!("Готовые наборы")),
        presets(st),
        label(&t!("Шрифт")),
        fonts(st),
        label(&t!("Насыщенность")),
        Box::new(
            SegmentedButton::new(vec![t!("Обычный"), t!("Средний"), t!("Полужирный"), t!("Жирный")])
                .selected(WEIGHTS.iter().position(|w| *w == s.weight).unwrap_or(2))
                .on_change(move |i| update(st, |s| s.weight = WEIGHTS[i.min(3)])),
        ),
        slider(st, &t!("Размер"), s.size, 3.0, 10.0, 0.1, pct1, |s| s.size, |s, v| s.size = v),
        hint(&t!("Доля высоты кадра: в окне и во весь экран субтитры одного вида.")),
        label(&t!("Цвет текста")),
        swatches(st, COLORS, |s| &s.color, |s, c| s.color = c.into()),
        slider(st, &t!("Непрозрачность текста"), s.opacity, 30.0, 100.0, 1.0, pct, |s| s.opacity, |s, v| s.opacity = v),
        label(&t!("Контур")),
        swatches(st, OUTLINE_COLORS, |s| &s.outline_color, |s, c| s.outline_color = c.into()),
        slider(st, &t!("Толщина контура"), s.outline, 0.0, 16.0, 0.5, pct1, |s| s.outline, |s, v| s.outline = v),
        slider(st, &t!("Мягкость контура"), s.softness, 0.0, 8.0, 0.5, pct1, |s| s.softness, |s, v| s.softness = v),
        label(&t!("Тень")),
        slider(st, &t!("Насыщенность тени"), s.shadow, 0.0, 100.0, 1.0, pct, |s| s.shadow, |s, v| s.shadow = v),
        slider(st, &t!("Размытие тени"), s.shadow_blur, 0.0, 40.0, 0.5, pct1, |s| s.shadow_blur, |s, v| s.shadow_blur = v),
        slider(st, &t!("Сдвиг тени"), s.shadow_offset, 0.0, 15.0, 0.5, pct1, |s| s.shadow_offset, |s, v| s.shadow_offset = v),
        label(&t!("Плашка под текстом")),
        swatches(st, BOX_COLORS, |s| &s.box_color, |s, c| s.box_color = c.into()),
        slider(st, &t!("Непрозрачность плашки"), s.box_opacity, 0.0, 100.0, 1.0, pct, |s| s.box_opacity, |s, v| s.box_opacity = v),
        label(&t!("Положение")),
        Box::new(
            SegmentedButton::new(vec![t!("По кадру"), t!("По окну")])
                .selected(if s.in_frame { 0 } else { 1 })
                .on_change(move |i| update(st, |s| s.in_frame = i == 0)),
        ),
        hint(&t!("«По окну» — у широкого кадра субтитры лягут на чёрное поле под ним.")),
        slider(st, &t!("Отступ от края"), s.margin, 0.0, 25.0, 0.5, pct1, |s| s.margin, |s, v| s.margin = v),
        slider(st, &t!("Ширина строки"), s.max_width, 40.0, 98.0, 1.0, pct, |s| s.max_width, |s, v| s.max_width = v),
        slider(st, &t!("Межстрочный интервал"), s.line_spacing, 0.9, 1.6, 0.01, times, |s| s.line_spacing, |s, v| s.line_spacing = v),
        label(&t!("Появление")),
        Box::new(
            SegmentedButton::new(anim_segs(false))
                .selected(ANIMS.iter().position(|a| *a == s.anim_in).unwrap_or(2))
                .on_change(move |i| update(st, |s| s.anim_in = ANIMS[i.min(3)].into())),
        ),
        slider(st, &t!("Длительность появления"), s.anim_in_ms as f32, 60.0, 600.0, 10.0, ms, |s| s.anim_in_ms as f32, |s, v| s.anim_in_ms = v as u32),
        label(&t!("Исчезновение")),
        Box::new(
            SegmentedButton::new(anim_segs(true))
                .selected(ANIMS.iter().position(|a| *a == s.anim_out).unwrap_or(1))
                .on_change(move |i| update(st, |s| s.anim_out = ANIMS[i.min(3)].into())),
        ),
        slider(st, &t!("Длительность исчезновения"), s.anim_out_ms as f32, 60.0, 600.0, 10.0, ms, |s| s.anim_out_ms as f32, |s, v| s.anim_out_ms = v as u32),
        toggle_row(&t!("Цвета из файла"), &t!("Цветные реплики из SRT и ASS."), s.file_colors, move |on| update(st, |s| s.file_colors = on)),
        toggle_row(
            &t!("Надписи на своих местах"),
            &t!("Позиции из ASS: перевод вывески — у самой вывески."),
            s.file_positions,
            move |on| update(st, |s| s.file_positions = on),
        ),
        Box::new(
            Button::new(t!("Сбросить оформление"))
                .icon(icons::UNDO)
                .on_click(move || {
                    update_all(st, |s| {
                        let keep = (s.auto, s.language.clone());
                        *s = SubSettings::default();
                        (s.auto, s.language) = keep;
                    })
                }),
        ),
    ];
    card(icons::PALETTE, &t!("Оформление"), rows)
}

fn toggle_row(title: &str, sub: &str, on: bool, f: impl FnMut(bool) + Send + 'static) -> W {
    Box::new(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Column::new().gap(2.0).child(Text::new(title).max_lines(2).class("set-row-title")).child(Text::new(sub).max_lines(3).class("set-hint")).class("grow"))
            .child(Toggle::with_state(on).on_change(f)),
    )
}

fn auto_card(st: St) -> W {
    let s = st.subs.get_untracked();
    let items: Vec<DropdownItem> = LANGUAGES.iter().map(|c| DropdownItem::new(*c, language_name(c))).collect();
    let rows: Vec<W> = vec![
        Box::new(
            SegmentedButton::new(vec![t!("Вручную"), t!("Чужая речь"), t!("Всегда")])
                .selected(match s.auto {
                    Auto::Off => 0,
                    Auto::Foreign => 1,
                    Auto::Always => 2,
                })
                .on_change(move |i| {
                    update(st, |s| {
                        s.auto = match i {
                            0 => Auto::Off,
                            2 => Auto::Always,
                            _ => Auto::Foreign,
                        }
                    })
                }),
        ),
        hint(&t!("«Чужая речь»: озвучка на другом языке — субтитры на вашем; своя озвучка — только надписи (если есть такая дорожка). Субтитры, положенные рядом с роликом, включаются сами.")),
        label(&t!("Язык субтитров")),
        Box::new(Dropdown::with_items(items).selected(s.language.clone()).on_change(move |v| {
            let v = v.to_string();
            update(st, |s| s.language = v)
        })),
    ];
    card(icons::AUTO, &t!("Включать автоматически"), rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(i: usize, lang: Option<&str>, forced: bool, default: bool) -> SubtitleTrackInfo {
        SubtitleTrackInfo {
            source: SubtitleSource::Embedded(i),
            language: lang.map(String::from),
            title: None,
            codec: "SRT".into(),
            bitmap: false,
            default,
            forced,
            hearing_impaired: false,
        }
    }

    #[test]
    fn auto_pick_follows_audio_language() {
        let s = SubSettings { language: "ru".into(), ..Default::default() };
        let tracks = vec![tr(3, Some("rus"), true, false), tr(4, Some("rus"), false, false), tr(5, Some("eng"), false, true)];
        // английская озвучка — полные русские
        assert_eq!(pick(&tracks, Some("eng"), &s, None), Some(SubtitleSource::Embedded(4)));
        // русская озвучка — только надписи
        assert_eq!(pick(&tracks, Some("rus"), &s, None), Some(SubtitleSource::Embedded(3)));
        // выбор сеанса: выключено
        assert_eq!(pick(&tracks, Some("eng"), &s, Some(&SubPref::Off)), None);
        // выбор сеанса: английские
        let p = SubPref::Track { title: None, language: Some("eng".into()), forced: false };
        assert_eq!(pick(&tracks, Some("rus"), &s, Some(&p)), Some(SubtitleSource::Embedded(5)));
        // вручную — ничего
        let off = SubSettings { auto: Auto::Off, ..s.clone() };
        assert_eq!(pick(&tracks, Some("eng"), &off, None), None);
    }

    #[test]
    fn external_file_shows_when_language_unknown() {
        let s = SubSettings { language: "ru".into(), ..Default::default() };
        let mut ext = tr(0, None, false, false);
        ext.source = SubtitleSource::External("/v/a.srt".into());
        assert_eq!(pick(&[ext.clone()], None, &s, None), Some(ext.source.clone()));
    }

    #[test]
    fn presets_round_trip() {
        let mut s = SubSettings::default();
        assert!(PRESETS[0].matches(&s));
        PRESETS[1].apply(&mut s);
        assert!(PRESETS[1].matches(&s) && !PRESETS[0].matches(&s));
        let st = s.style();
        assert!(st.box_opacity > 0.5 && st.outline == 0.0);
    }
}
