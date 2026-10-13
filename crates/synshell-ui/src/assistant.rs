use std::result::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use syngui::prelude::*;
use syngui::widgets::visual::canvas::Canvas;
use syngui::widgets::visual::MarkdownView;
use syngui::widgets::{Stack, Tooltip};
use synassist::agent::{Agent, AgentEvent, DaemonBackend};
use synassist::chat::{Chat, Role, Store};
use synassist_voice::{Phase, VisHandle, VoiceConfig, VoiceEvent};

use crate::ui::{icon, rx};
use crate::ctx::PopupKind;
use crate::ShellCtx;

const SPARKLE: &str = "\u{E65F}";
const MIC: &str = "\u{E029}";
const MIC_OFF: &str = "\u{E02B}";
const STOP: &str = "\u{E047}";
const VOICE_CHAT: &str = "\u{E62E}";
const KEYBOARD: &str = "\u{E312}";
const CALL_END: &str = "\u{E0B1}";
const VOLUME_UP: &str = "\u{E050}";
const COPY: &str = "\u{E14D}";
const OPEN: &str = "\u{E89E}";
const ADD: &str = "\u{E145}";
const SEND: &str = "\u{E163}";

#[derive(Clone, Copy)]
struct Panel {
    query: RwSignal<String>,
    answer: RwSignal<String>,
    tool: RwSignal<Option<String>>,
    confirm: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    chat_id: RwSignal<Option<String>>,
    input_rev: RwSignal<u64>,
    voice_open: RwSignal<bool>,
    phase: RwSignal<Phase>,
    vis: RwSignal<Option<VisHandle>>,
    heard: RwSignal<String>,
    muted: RwSignal<bool>,
    dictating: RwSignal<bool>,
    speaking: RwSignal<bool>,
    note: RwSignal<Option<String>>,
}

thread_local! {
    static PANEL: std::cell::Cell<Option<Panel>> = const { std::cell::Cell::new(None) };
}

struct Shared {
    cancel: Arc<AtomicBool>,
    answer: Mutex<Option<Sender<bool>>>,
    turn: Mutex<Option<Sender<Result<String, String>>>>,
}

fn shared() -> &'static Shared {
    static S: std::sync::OnceLock<Shared> = std::sync::OnceLock::new();
    S.get_or_init(|| Shared { cancel: Arc::new(AtomicBool::new(false)), answer: Mutex::new(None), turn: Mutex::new(None) })
}

fn panel() -> Panel {
    PANEL.with(|p| match p.get() {
        Some(v) => v,
        None => {
            let v = Panel {
                query: use_signal(String::new()),
                answer: use_signal(String::new()),
                tool: use_signal(None),
                confirm: use_signal(None),
                busy: use_signal(false),
                chat_id: use_signal(None),
                input_rev: use_signal(0),
                voice_open: use_signal(false),
                phase: use_signal(Phase::Off),
                vis: use_signal(None),
                heard: use_signal(String::new()),
                muted: use_signal(false),
                dictating: use_signal(false),
                speaking: use_signal(false),
                note: use_signal(None),
            };
            p.set(Some(v));
            v
        }
    })
}

fn popup_open() -> bool {
    ShellCtx::get().popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::Assistant))
}

fn reply(p: Panel, yes: bool) {
    if let Some(tx) = shared().answer.lock().unwrap().take() {
        let _ = tx.send(yes);
    }
    p.confirm.set(None);
}

fn stop(p: Panel) {
    shared().cancel.store(true, Ordering::Relaxed);
    reply(p, false);
}

fn ask(p: Panel, text: String, voice: bool) {
    let text = text.trim().to_string();
    if text.is_empty() || p.busy.get_untracked() {
        if voice {
            if let Some(tx) = shared().turn.lock().unwrap().take() {
                let _ = tx.send(Err(String::new()));
            }
        }
        return;
    }
    p.busy.set(true);
    p.answer.set(String::new());
    p.tool.set(None);
    p.query.set(String::new());
    p.input_rev.set(p.input_rev.get_untracked() + 1);
    shared().cancel.store(false, Ordering::Relaxed);
    let store = Store::open();
    let mut chat = p.chat_id.get_untracked().and_then(|id| store.load(&id)).unwrap_or_else(Chat::new);
    let from = chat.messages.len();
    let speak_after = !voice && ShellCtx::get().cfg().assistant.speak_answers;
    std::thread::Builder::new()
        .name("assistant".into())
        .spawn(move || {
            let cfg = synshell_common::Config::load().0.assistant;
            let mut agent = Agent::configured(Arc::new(DaemonBackend), &cfg.disabled_tools, cfg.auto_approve, cfg.autocompact);
            if voice {
                agent.system = Some(synassist::prompt::build_voice(&agent.tools));
            }
            let confirm = move |_: &str, summary: &str| -> bool {
                let (tx, rx) = channel();
                *shared().answer.lock().unwrap() = Some(tx);
                let s = summary.to_string();
                run_on_main_thread(move || p.confirm.set(Some(s)));
                rx.recv().unwrap_or(false)
            };
            let cancel = shared().cancel.clone();
            let r = agent.run_turn(&mut chat, &text, &confirm, &cancel, &mut |ev| {
                run_on_main_thread(move || match ev {
                    AgentEvent::Text(t) => {
                        let mut a = p.answer.get_untracked();
                        a.push_str(&t);
                        p.answer.set(a);
                    }
                    AgentEvent::ToolStart { summary, .. } => p.tool.set(Some(summary)),
                    AgentEvent::Message(m) if m.role == Role::Assistant && !m.tool_calls.is_empty() => p.answer.set(String::new()),
                    _ => {}
                });
            });
            let _ = store.save(&chat);
            let id = chat.id.clone();
            let answer: String = chat
                .messages
                .iter()
                .skip(from)
                .filter(|m| m.role == Role::Assistant && !m.text.trim().is_empty())
                .map(|m| m.text.trim().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let result = match &r {
                Ok(()) => Ok(answer.clone()),
                Err(e) => Err(e.clone()),
            };
            if let Some(tx) = shared().turn.lock().unwrap().take() {
                let _ = tx.send(result);
            }
            run_on_main_thread(move || {
                if let Err(e) = r {
                    p.answer.set(format!("**Ошибка:** {e}"));
                }
                p.chat_id.set(Some(id));
                p.tool.set(None);
                p.busy.set(false);
                p.confirm.set(None);
                if speak_after && !answer.is_empty() {
                    speak(p, synassist_voice::strip_markdown(&answer));
                }
            });
        })
        .ok();
}

fn listener(p: Panel) -> synassist_voice::Listener {
    Arc::new(move |ev: VoiceEvent| {
        run_on_main_thread(move || {
            if !popup_open() && synassist_voice::conversation_running() {
                synassist_voice::stop_conversation();
            }
            match ev {
                VoiceEvent::Phase(ph) => p.phase.set(ph),
                VoiceEvent::Vis(v) => p.vis.set(v),
                VoiceEvent::Heard(t) => p.heard.set(t),
                VoiceEvent::Speaking(on) => p.speaking.set(on),
                VoiceEvent::Error(e) => p.note.set(Some(e)),
                VoiceEvent::Stopped => {
                    if p.note.get_untracked().is_none() {
                        p.voice_open.set(false);
                    }
                }
            }
        });
    })
}

fn turn(p: Panel) -> synassist_voice::Turn {
    Arc::new(move |text: String| -> Result<String, String> {
        let (tx, rx) = channel::<Result<String, String>>();
        *shared().turn.lock().unwrap() = Some(tx);
        run_on_main_thread(move || ask(p, text, true));
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(r) => return r,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return Err(String::new()),
            }
        }
    })
}

fn start_voice(p: Panel) {
    p.voice_open.set(true);
    p.note.set(None);
    p.heard.set(String::new());
    p.muted.set(false);
    synassist_voice::set_muted(false);
    synassist_voice::start_conversation(VoiceConfig::load(), turn(p), listener(p));
}

fn stop_voice(p: Panel) {
    synassist_voice::stop_conversation();
    if let Some(tx) = shared().turn.lock().unwrap().take() {
        let _ = tx.send(Err(String::new()));
    }
    p.voice_open.set(false);
    p.note.set(None);
}

fn speak(p: Panel, text: String) {
    p.speaking.set(true);
    synassist_voice::speak(
        VoiceConfig::load(),
        text,
        Arc::new(move |_: Result<(), String>| run_on_main_thread(move || p.speaking.set(false))),
    );
}

fn toggle_dictation(p: Panel) {
    if synassist_voice::dictating() {
        synassist_voice::toggle_dictation(VoiceConfig::load(), listener(p), Arc::new(|_| {}));
        return;
    }
    p.dictating.set(true);
    let started = synassist_voice::toggle_dictation(
        VoiceConfig::load(),
        listener(p),
        Arc::new(move |r: Result<String, String>| {
            run_on_main_thread(move || {
                p.dictating.set(false);
                match r {
                    Ok(t) if !t.is_empty() => {
                        let prev = p.query.get_untracked();
                        p.query.set(if prev.trim().is_empty() { t } else { format!("{} {t}", prev.trim_end()) });
                        p.input_rev.set(p.input_rev.get_untracked() + 1);
                    }
                    Ok(_) => {}
                    Err(e) => p.note.set(Some(e)),
                }
            });
        }),
    );
    if !started {
        p.dictating.set(false);
    }
}

fn open_window(p: Panel) {
    let id = p.chat_id.get_untracked();
    let cmd = match id {
        Some(id) => format!("synassistant --chat {id}"),
        None => "synassistant".into(),
    };
    crate::actions::spawn(&cmd);
    ShellCtx::get().close_popup();
}

fn tip(w: impl Widget + 'static, text: impl Into<String>) -> Box<dyn Widget> {
    Box::new(Tooltip::new(w, text).delay_ms(500))
}

fn aura(handle: Option<VisHandle>, phase: Phase) -> Box<dyn Widget> {
    let prev: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(vec![0.0; 24]));
    Box::new(
        Canvas::new(move |ctx, t| {
            let accent = ctx.mss_accent().unwrap_or_else(|| Color::from_hex("#3B82F6"));
            let (cx, cy) = (ctx.width() * 0.5, ctx.height() * 0.5);
            let inner = 30.0;
            let outer = ctx.width().min(ctx.height()) * 0.5 - 4.0;
            let target = match (&handle, phase) {
                (Some(h), Phase::Listening) => h.snapshot_bars(24),
                _ => vec![0.0; 24],
            };
            let mut bars = prev.lock().unwrap_or_else(|e| e.into_inner());
            for (i, v) in target.iter().enumerate() {
                bars[i] += (v - bars[i]) * 0.3;
            }
            match phase {
                Phase::Listening => {
                    let n = bars.len() as f32;
                    for (i, v) in bars.iter().enumerate() {
                        let a = (i as f32 / n) * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                        let r0 = inner + 6.0;
                        let r1 = r0 + (outer - r0 - 2.0) * v.clamp(0.0, 1.0);
                        ctx.set_stroke_width(3.0);
                        ctx.set_color(accent.with_alpha(0.5 + v.clamp(0.0, 1.0) * 0.5));
                        ctx.draw_line(cx + a.cos() * r0, cy + a.sin() * r0, cx + a.cos() * r1, cy + a.sin() * r1);
                    }
                }
                Phase::Speaking => {
                    for i in 0..3 {
                        let p = ((t / 1.5) + i as f32 / 3.0).fract();
                        ctx.set_stroke_width(2.0 + (1.0 - p) * 3.0);
                        ctx.set_color(accent.with_alpha((1.0 - p) * 0.6));
                        ctx.stroke_circle(cx, cy, inner + (outer - inner) * p);
                    }
                }
                Phase::Off => {}
                _ => {
                    for i in 0..2 {
                        let start = t * 2.0 + i as f32 * 2.6;
                        ctx.set_stroke_width(3.0);
                        ctx.set_color(accent.with_alpha(0.75));
                        ctx.draw_arc(cx, cy, inner + 10.0 + i as f32 * 10.0, start, start + 1.4);
                    }
                }
            }
            ctx.set_color(accent.with_alpha(0.25));
            ctx.fill_circle(cx, cy, inner + 4.0);
            ctx.set_color(accent);
            ctx.fill_circle(cx, cy, inner - 2.0);
        })
        .size(140.0, 140.0)
        .animated(true)
        .class("assist-aura"),
    )
}

fn voice_stage(p: Panel) -> Box<dyn Widget> {
    let stage = rx(move || {
        let handle = p.vis.get();
        let phase = p.phase.get();
        let muted = p.muted.get();
        let (glyph, label) = match phase {
            Phase::Speaking => (STOP, t!("Остановить озвучку")),
            Phase::Listening if muted => (MIC_OFF, t!("Включить микрофон")),
            Phase::Listening => (MIC, t!("Выключить микрофон")),
            Phase::Thinking => (STOP, t!("Прервать ответ")),
            _ => (MIC, t!("Подождите…")),
        };
        let btn = Button::new("").icon(glyph).class("btn assist-center").on_click(move || match phase {
            Phase::Speaking => synassist_voice::stop_speaking(),
            Phase::Listening => {
                synassist_voice::set_muted(!muted);
                p.muted.set(!muted);
            }
            Phase::Thinking => stop(p),
            _ => {}
        });
        Box::new(
            DecoratedBox::new().class("assist-aura-wrap").child(
                Stack::new()
                    .clip(false)
                    .child(aura(handle, phase))
                    .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("assist-aura-wrap").child(tip(btn, label))),
            ),
        )
    });
    let status = rx(move || {
        let phase = p.phase.get();
        let muted = p.muted.get();
        let text = if let Some(n) = p.note.get() {
            n
        } else {
            match phase {
                Phase::Off => t!("Голосовой режим выключен").to_string(),
                Phase::Starting => t!("Включаю микрофон…").to_string(),
                Phase::Listening if muted => t!("Микрофон выключен").to_string(),
                Phase::Listening => t!("Слушаю…").to_string(),
                Phase::Transcribing => t!("Распознаю…").to_string(),
                Phase::Thinking => t!("Думаю…").to_string(),
                Phase::Speaking => t!("Говорю…").to_string(),
            }
        };
        Box::new(Text::new(text).max_lines(2).class("assist-voice-status"))
    });
    let transcript = rx(move || {
        let heard = p.heard.get();
        let answer = p.answer.get();
        let tool = p.tool.get();
        let confirm = p.confirm.get();
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if !heard.is_empty() {
            col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(DecoratedBox::new().class("assist-heard").child(Text::new(heard).class("assist-heard-text"))));
        }
        if let Some(s) = confirm {
            col = col.child(confirm_card(p, s));
        } else if let Some(t) = tool.filter(|_| p.busy.get() && answer.is_empty()) {
            col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon("\u{E869}").class("assist-tool-icon")).child(Text::new(t).max_lines(1).class("assist-tool grow")));
        }
        if !answer.is_empty() {
            col = col.child(ScrollView::new().vertical().follow_end(true).class("assist-voice-scroll").child(MarkdownView::new(answer).selectable(true).class("assist-answer")));
        }
        Box::new(col)
    });
    let actions = rx(move || {
        let muted = p.muted.get();
        let speaking = p.speaking.get();
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::Center);
        row = row.child(tip(
            Button::new("").icon(if muted { MIC_OFF } else { MIC }).class(if muted { "btn assist-act off" } else { "btn assist-act" }).on_click(move || {
                synassist_voice::set_muted(!muted);
                p.muted.set(!muted);
            }),
            if muted { t!("Включить микрофон") } else { t!("Выключить микрофон") },
        ));
        if speaking {
            row = row.child(tip(Button::new("").icon(STOP).class("btn assist-act").on_click(|| synassist_voice::stop_speaking()), t!("Замолчать")));
        }
        row = row.child(tip(Button::new("").icon(KEYBOARD).class("btn assist-act").on_click(move || stop_voice(p)), t!("Вернуться к тексту")));
        row = row.child(tip(Button::new("").icon(CALL_END).class("btn assist-act end").on_click(move || stop_voice(p)), t!("Завершить")));
        Box::new(row)
    });
    Box::new(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).class("assist-voice").child(stage).child(status).child(DecoratedBox::new().class("assist-voice-body").child(transcript)).child(actions))
}

/// Индикатор детектора речи (как проверка микрофона в synspeak): полоса уровня
/// с отметкой порога тишины, цифры и что детектор слышит.
fn confirm_card(p: Panel, summary: String) -> Column {
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("assist-confirm")
        .child(Text::new(t!("Разрешить действие?")).class("assist-confirm-title"))
        .child(Text::new(summary).max_lines(4).class("assist-confirm-sum"))
        .child(
            Row::new()
                .gap(8.0)
                .main_axis_alignment(MainAxisAlignment::End)
                .child(Button::new(t!("Отклонить")).class("btn").on_click(move || reply(p, false)))
                .child(Button::new(t!("Разрешить")).class("btn btn-primary").on_click(move || reply(p, true))),
        )
}

fn header(p: Panel) -> Box<dyn Widget> {
    Box::new(rx(move || {
        p.input_rev.get();
        let q = p.query;
        let initial = q.get_untracked();
        let mic = rx(move || {
            let on = p.dictating.get();
            let phase = p.phase.get();
            let label = if on && phase == Phase::Transcribing { t!("Распознаю…") } else if on { t!("Говорите… нажмите ещё раз, чтобы закончить") } else { t!("Надиктовать") };
            tip(Button::new("").icon(if on { STOP } else { MIC }).class(if on { "btn assist-mic on" } else { "btn assist-mic" }).on_click(move || toggle_dictation(p)), label)
        });
        let send = rx(move || {
            if p.busy.get() {
                tip(Button::new("").icon(STOP).class("btn assist-mic stop").on_click(move || stop(p)), t!("Остановить"))
            } else {
                tip(Button::new("").icon(SEND).class("btn assist-mic send").on_click(move || ask(p, p.query.get_untracked(), false)), t!("Отправить"))
            }
        });
        Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("assist-brand").child(icon(SPARKLE).class("assist-brand-ic")))
                .child(
                    TextField::new()
                        .text(initial)
                        .placeholder(t!("Спросите ассистента…"))
                        .autofocus(true)
                        .on_change(move |t| q.set(t.to_string()))
                        .on_submit(move |t| ask(p, t.to_string(), false))
                        .class("search-field grow"),
                )
                .child(mic)
                .child(send)
                .child(tip(Button::new("").icon(VOICE_CHAT).class("btn assist-mic voice").on_click(move || start_voice(p)), t!("Голосовой разговор"))),
        )
    }))
}

pub fn view(_ctx: ShellCtx) -> impl Widget {
    let p = panel();
    let body = rx(move || {
        if p.voice_open.get() {
            return voice_stage(p);
        }
        let answer = p.answer.get();
        let busy = p.busy.get();
        let tool = p.tool.get();
        let confirm = p.confirm.get();
        let speaking = p.speaking.get();
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Some(s) = confirm {
            col = col.child(confirm_card(p, s));
        } else if let Some(t) = tool.filter(|_| busy && answer.is_empty()) {
            col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon("\u{E869}").class("assist-tool-icon")).child(Text::new(t).max_lines(1).class("assist-tool grow")));
        } else if busy && answer.is_empty() {
            col = col.child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(16.0).stroke_width(2.0).class("assist-spin")).child(Text::new(t!("Думаю…")).class("assist-hint")));
        }
        if !answer.is_empty() {
            col = col.child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .class("assist-msg")
                    .child(DecoratedBox::new().class("assist-avatar").child(icon(SPARKLE).class("assist-avatar-ic")))
                    .child(ScrollView::new().vertical().follow_end(true).class("assist-scroll grow").child(MarkdownView::new(answer.clone()).with_copy_code(true).selectable(true).class("assist-answer"))),
            );
        }
        if !busy && p.chat_id.get().is_some() {
            let a1 = answer.clone();
            let a2 = answer.clone();
            let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
            if !answer.is_empty() {
                row = row.child(tip(Button::new("").icon(COPY).class("btn assist-act small").on_click(move || syngui::clipboard::copy(&a1)), t!("Копировать")));
                if speaking {
                    row = row.child(tip(Button::new("").icon(STOP).class("btn assist-act small").on_click(|| synassist_voice::stop_speaking()), t!("Остановить озвучку")));
                } else {
                    row = row.child(tip(Button::new("").icon(VOLUME_UP).class("btn assist-act small").on_click(move || speak(p, synassist_voice::strip_markdown(&a2))), t!("Прочитать вслух")));
                }
            }
            row = row
                .child(DecoratedBox::new().class("grow"))
                .child(Button::new(t!("Новый вопрос")).icon(ADD).class("btn assist-ghost").on_click(move || {
                    p.chat_id.set(None);
                    p.answer.set(String::new());
                    p.heard.set(String::new());
                    p.input_rev.set(p.input_rev.get_untracked() + 1);
                }))
                .child(Button::new(t!("Открыть чат")).icon(OPEN).class("btn btn-primary").on_click(move || open_window(p)));
            if !answer.is_empty() {
                col = col.child(DecoratedBox::new().class("assist-sep"));
            }
            col = col.child(row);
        }
        if col_is_empty(busy, &p) {
            return Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(t!("Управление системой, файлы, интернет и медиа — на нейросетях этого устройства.")).max_lines(2).class("assist-hint grow"))
                    .child(Text::new(t!("Esc — закрыть")).class("assist-hint dim")),
            ) as Box<dyn Widget>;
        }
        Box::new(col)
    });
    Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(DecoratedBox::new().child(header(p)).class("search-box assist-box"))
        .child(AnimatedSize::new(body).axis(AnimationAxis::Height).spring(420.0, 40.0).class("popup-morph"))
}

fn col_is_empty(busy: bool, p: &Panel) -> bool {
    !busy && p.answer.get_untracked().is_empty() && p.confirm.get_untracked().is_none() && p.chat_id.get_untracked().is_none()
}
