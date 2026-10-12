use std::result::Result;
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use syngui::prelude::*;
pub use synassist_voice::strip_markdown;
use synassist_voice::{VoiceConfig, VoiceEvent};

use crate::state::{shared, St};

fn listener(st: St) -> synassist_voice::Listener {
    Arc::new(move |ev: VoiceEvent| {
        run_on_main_thread(move || match ev {
            VoiceEvent::Phase(p) => st.voice.phase.set(p),
            VoiceEvent::Vis(v) => st.voice.vis.set(v),
            VoiceEvent::Level(s) => st.voice.vad.set(Some(s)),
            VoiceEvent::Heard(t) => st.voice.heard.set(t),
            VoiceEvent::Speaking(on) => st.voice.speaking.set(on),
            VoiceEvent::Error(e) => {
                if st.voice.open.get_untracked() {
                    st.voice.note.set(Some(e));
                } else {
                    st.toast(e);
                }
            }
            VoiceEvent::Stopped => {
                if st.voice.note.get_untracked().is_none() {
                    st.voice.open.set(false);
                }
            }
        });
    })
}

fn turn(st: St) -> synassist_voice::Turn {
    Arc::new(move |text: String| -> Result<String, String> {
        let (tx, rx) = channel::<Result<String, String>>();
        *shared().turn.lock().unwrap() = Some(tx);
        run_on_main_thread(move || st.send_with(text, true));
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(r) => return r,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err(String::new()),
            }
        }
    })
}

pub fn start_conversation(st: St) {
    st.voice.open.set(true);
    st.voice.note.set(None);
    st.voice.heard.set(String::new());
    synassist_voice::set_muted(false);
    st.voice.muted.set(false);
    synassist_voice::start_conversation(VoiceConfig::load(), turn(st), listener(st));
}

pub fn stop_conversation(st: St) {
    synassist_voice::stop_conversation();
    if let Some(tx) = shared().turn.lock().unwrap().take() {
        let _ = tx.send(Err(String::new()));
    }
    st.voice.open.set(false);
    st.voice.note.set(None);
}

pub fn set_muted(st: St, muted: bool) {
    synassist_voice::set_muted(muted);
    st.voice.muted.set(muted);
}

pub fn stop_speaking() {
    synassist_voice::stop_speaking();
}

pub fn speak(st: St, text: String) {
    st.voice.speaking.set(true);
    synassist_voice::speak(
        VoiceConfig::load(),
        text,
        Arc::new(move |r: Result<(), String>| {
            run_on_main_thread(move || {
                st.voice.speaking.set(false);
                if let Err(e) = r {
                    st.toast(t!("Озвучка: {e}", e = e));
                }
            });
        }),
    );
}

pub fn toggle_dictation(st: St) {
    if synassist_voice::dictating() {
        synassist_voice::toggle_dictation(VoiceConfig::load(), listener(st), Arc::new(|_| {}));
        return;
    }
    st.voice.dictating.set(true);
    let started = synassist_voice::toggle_dictation(
        VoiceConfig::load(),
        listener(st),
        Arc::new(move |r: Result<String, String>| {
            run_on_main_thread(move || {
                st.voice.dictating.set(false);
                match r {
                    Ok(t) if !t.is_empty() => shared().insert.lock().unwrap().push(t),
                    Ok(_) => {}
                    Err(e) => st.toast(t!("Распознавание: {e}", e = e)),
                }
            });
        }),
    );
    if !started {
        st.voice.dictating.set(false);
    }
}

pub fn shutdown() {
    synassist_voice::shutdown();
}
