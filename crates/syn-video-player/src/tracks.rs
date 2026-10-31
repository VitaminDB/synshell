//! Звуковые дорожки (озвучки): кнопка в шапке плеера, если дорожек больше одной, и панель выбора.
//! Выбранная озвучка запоминается до конца сеанса: следующий ролик (серия) откроется с дорожкой
//! того же названия, а нет такой — того же языка.

use syngui::prelude::*;
use syngui::video::{AudioStreamInfo, VideoPlayer};
use syngui::widgets::*;

use crate::ui::{icons, St};

type W = Box<dyn Widget>;

/// Какую озвучку выбрал пользователь.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioPref {
    pub title: Option<String>,
    pub language: Option<String>,
}

impl AudioPref {
    fn of(t: &AudioStreamInfo) -> Self {
        Self { title: t.title.clone(), language: t.language.clone() }
    }
}

/// Дорожка под запомненный выбор: то же название (и язык, если он есть), иначе тот же язык.
fn pick(tracks: &[AudioStreamInfo], current: Option<usize>, pref: &AudioPref) -> Option<usize> {
    let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
        (Some(a), Some(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
        _ => false,
    };
    if pref.title.is_some() {
        let by_title = tracks
            .iter()
            .find(|t| same(&t.title, &pref.title) && (pref.language.is_none() || t.language == pref.language))
            .or_else(|| tracks.iter().find(|t| same(&t.title, &pref.title)));
        if let Some(t) = by_title {
            return Some(t.stream_index);
        }
    }
    // Названия нет — только язык; текущая дорожка уже на нём — оставить её.
    let cur = tracks.iter().find(|t| Some(t.stream_index) == current);
    if pref.language.is_some() && !cur.is_some_and(|c| same(&c.language, &pref.language)) {
        return tracks.iter().find(|t| same(&t.language, &pref.language)).map(|t| t.stream_index);
    }
    None
}

/// Только что открытый ролик: включить запомненную озвучку.
pub fn apply_pref(st: St, p: &mut VideoPlayer) {
    let Some(pref) = st.audio_pref.get_untracked() else { return };
    if let Some(idx) = pick(p.audio_tracks(), p.audio_track(), &pref) {
        if Some(idx) != p.audio_track() {
            if let Err(e) = p.set_audio_track(idx) {
                tracing::warn!("звуковая дорожка {idx}: {e}");
            }
        }
    }
}

fn language_name(code: &str) -> String {
    let name = match code.to_ascii_lowercase().as_str() {
        "rus" | "ru" => t!("Русский"),
        "eng" | "en" => t!("Английский"),
        "ukr" | "uk" => t!("Украинский"),
        "bel" | "be" => t!("Белорусский"),
        "kaz" | "kk" => t!("Казахский"),
        "ger" | "deu" | "de" => t!("Немецкий"),
        "fre" | "fra" | "fr" => t!("Французский"),
        "spa" | "es" => t!("Испанский"),
        "ita" | "it" => t!("Итальянский"),
        "por" | "pt" => t!("Португальский"),
        "pol" | "pl" => t!("Польский"),
        "jpn" | "ja" => t!("Японский"),
        "kor" | "ko" => t!("Корейский"),
        "chi" | "zho" | "zh" => t!("Китайский"),
        "tur" | "tr" => t!("Турецкий"),
        _ => return code.to_string(),
    };
    name.to_string()
}

fn channels_label(ch: u16) -> String {
    match ch {
        1 => t!("моно").to_string(),
        2 => t!("стерео").to_string(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => t!("{n} кан.", n = n).to_string(),
    }
}

/// Строка дорожки: название (или язык), под ним «Русский · AC-3 · 5.1».
fn labels(t: &AudioStreamInfo, n: usize) -> (String, String) {
    let lang = t.language.as_deref().map(language_name);
    let title = t.title.clone().or_else(|| lang.clone()).unwrap_or_else(|| t!("Дорожка {n}", n = n).to_string());
    let mut sub: Vec<String> = Vec::new();
    if let Some(l) = lang.filter(|l| *l != title) {
        sub.push(l);
    }
    sub.push(t.codec.clone());
    sub.push(channels_label(t.channels));
    (title, sub.join(" · "))
}

fn tracks_of(st: St) -> Option<(Vec<AudioStreamInfo>, Option<usize>)> {
    let p = st.playing.get_untracked()?;
    let pl = p.player.lock().ok()?;
    Some((pl.audio_tracks().to_vec(), pl.audio_track()))
}

/// Кнопка в шапке: есть из чего выбирать — показать.
pub fn button(st: St) -> W {
    match tracks_of(st) {
        Some((tracks, _)) if tracks.len() > 1 => Box::new(
            ToolButton::new(icons::VOICE)
                .tooltip(t!("Звуковая дорожка"))
                .on_click(move || st.tracks_open.set(true))
                .class("vp-btn"),
        ),
        _ => Box::new(Row::new()),
    }
}

fn select(st: St, t: &AudioStreamInfo) {
    st.tracks_open.set(false);
    let Some(p) = st.playing.get_untracked() else { return };
    let r = p.player.lock().map(|mut pl| pl.set_audio_track(t.stream_index));
    if let Ok(Err(e)) = r {
        tracing::warn!("звуковая дорожка {}: {e}", t.stream_index);
        return;
    }
    st.audio_pref.set(Some(AudioPref::of(t)));
    crate::subs::on_audio_changed(st, t.language.as_deref());
}

/// Панель выбора дорожки — справа, как настройки.
pub fn panel(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        if !st.tracks_open.get() {
            return vec![];
        }
        let Some((tracks, current)) = tracks_of(st) else { return vec![] };
        let close = move || st.tracks_open.set(false);
        let rows: Vec<W> = tracks.iter().enumerate().map(|(i, t)| row(st, t, i + 1, Some(t.stream_index) == current)).collect();
        let panel = Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(t!("Звуковая дорожка")).class("set-title"))
                    .child(DecoratedBox::new().class("grow"))
                    .child(ToolButton::new(icons::CLOSE).tooltip(t!("Закрыть")).on_click(close).class("set-close"))
                    .class("set-head"),
            )
            .child(
                ScrollView::new()
                    .vertical()
                    .child(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(rows).class("set-body"))
                    .class("grow"),
            )
            .class("set-panel");
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(close).child(DecoratedBox::new().class("set-scrim")))
                .child(
                    Row::new()
                        .main_axis_alignment(MainAxisAlignment::End)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(GestureDetector::new().on_click(|| {}).child(panel)),
                ),
        )]
    }))
}

fn row(st: St, t: &AudioStreamInfo, n: usize, active: bool) -> W {
    let (title, sub) = labels(t, n);
    let track = t.clone();
    Box::new(
        GestureDetector::new()
            .on_click(move || select(st, &track))
            .cursor(syngui::CursorIcon::Pointer)
            .child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(if active { icons::RADIO_ON } else { icons::RADIO_OFF }).class("track-radio"))
                            .child(
                                Column::new()
                                    .gap(2.0)
                                    .child(Text::new(title).max_lines(2).class("set-row-title"))
                                    .child(Text::new(sub).max_lines(2).class("set-hint"))
                                    .class("grow"),
                            ),
                    )
                    .class(if active { "track-row track-row-active" } else { "track-row" }),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(i: usize, lang: &str, title: &str) -> AudioStreamInfo {
        AudioStreamInfo {
            stream_index: i,
            language: Some(lang.into()),
            title: (!title.is_empty()).then(|| title.into()),
            codec: "AC-3".into(),
            channels: 6,
            sample_rate: 48000,
            default: i == 1,
        }
    }

    #[test]
    fn pick_follows_title_then_language() {
        let ts = vec![tr(1, "rus", "MovieDalen"), tr(4, "rus", "LostFilm"), tr(7, "eng", "Original")];
        let p = |t: &str, l: &str| AudioPref { title: (!t.is_empty()).then(|| t.into()), language: Some(l.into()) };
        assert_eq!(pick(&ts, Some(1), &p("lostfilm", "rus")), Some(4));
        // такого названия нет — по языку
        assert_eq!(pick(&ts, Some(1), &p("Кубик в кубе", "eng")), Some(7));
        // язык и так совпадает — не трогать
        assert_eq!(pick(&ts, Some(1), &p("Кубик в кубе", "rus")), None);
        assert_eq!(pick(&ts, Some(1), &p("", "eng")), Some(7));
    }

    #[test]
    fn labels_read_well() {
        let (t, s) = labels(&tr(4, "rus", "LostFilm"), 4);
        assert_eq!((t.as_str(), s.as_str()), ("LostFilm", "Русский · AC-3 · 5.1"));
        let (t, s) = labels(&tr(2, "eng", ""), 2);
        assert_eq!((t.as_str(), s.as_str()), ("Английский", "AC-3 · 5.1"));
    }
}
