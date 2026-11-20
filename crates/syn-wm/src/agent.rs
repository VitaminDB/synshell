//! Управление агентом (ИИ через synlink). synlink продлевает его каждым
//! действием (`Request::Agent { Start }`), без продления 15 с — гаснет само.
//! Индикатор рисует оболочка по `Event::Agent`. Полное управление
//! (`agent.toml` → `full_control`): физический ввод пользователя не доходит до
//! окон, пока агент управляет; Ctrl+Alt+Esc забирает управление, и 30 с
//! агент не может взять его снова.

use std::time::{Duration, Instant};

use smithay::backend::input::{InputBackend, InputEvent, KeyState, KeyboardKeyEvent};
use synshell_common::ipc::{AgentInfo, AgentOp, Event};
use synshell_tr::t;

use crate::state::State;

/// Без продления управление гаснет через столько.
pub const LEASE: Duration = Duration::from_secs(15);
/// Пользователь забрал управление — столько агент ждёт.
pub const COOLDOWN: Duration = Duration::from_secs(30);

pub struct AgentState {
    pub until: Instant,
    pub full: bool,
    pub reason: String,
}

// Коды клавиш xkb (evdev + 8).
const XKB_ESC: u32 = 9;
const XKB_CTRL: [u32; 2] = [37, 105];
const XKB_ALT: [u32; 2] = [64, 108];

impl State {
    pub fn agent_info(&self) -> AgentInfo {
        let now = Instant::now();
        let a = self.core.agent.as_ref().filter(|a| a.until > now);
        AgentInfo {
            active: a.is_some(),
            full: a.is_some_and(|a| a.full),
            reason: a.map(|a| a.reason.clone()).unwrap_or_default(),
            user_idle_ms: self.core.last_physical.elapsed().as_millis() as u64,
            cooldown_s: self.core.agent_cooldown.map(|c| c.saturating_duration_since(now).as_secs()).unwrap_or(0),
            takeback: synshell_common::agent::TAKEBACK.into(),
            full_allowed: synshell_common::agent::load().full_control,
        }
    }

    pub fn agent_op(&mut self, op: AgentOp) -> Result<AgentInfo, String> {
        match op {
            AgentOp::Status => {}
            AgentOp::Stop => self.agent_end(false),
            AgentOp::TakeBack => self.agent_end(true),
            AgentOp::Start { reason } => {
                let now = Instant::now();
                if let Some(c) = self.core.agent_cooldown.filter(|c| *c > now) {
                    return Err(t!("пользователь забрал управление — агент ждёт ещё {s} с", s = c.duration_since(now).as_secs().max(1)).into());
                }
                let full = synshell_common::agent::load().full_control;
                let was = self.core.agent.as_ref().filter(|a| a.until > now).map(|a| a.full);
                if was.is_none() {
                    tracing::info!(full, reason, "агент управляет");
                    self.core.agent_keys.clear();
                }
                self.core.agent = Some(AgentState { until: now + LEASE, full, reason });
                // Срок: проверить, не продлили ли.
                let _ = self.core.loop_handle.insert_source(
                    smithay::reexports::calloop::timer::Timer::from_duration(LEASE + Duration::from_millis(50)),
                    |_, _, state: &mut State| {
                        if state.core.agent.as_ref().is_some_and(|a| a.until <= Instant::now()) {
                            state.agent_end(false);
                        }
                        smithay::reexports::calloop::timer::TimeoutAction::Drop
                    },
                );
                if was != Some(full) {
                    self.agent_broadcast();
                }
            }
        }
        Ok(self.agent_info())
    }

    /// Управление кончилось: агент отдал, срок вышел или пользователь забрал (`takeback`).
    pub fn agent_end(&mut self, takeback: bool) {
        if self.core.agent.take().is_none() && !takeback {
            return;
        }
        if takeback {
            tracing::info!("пользователь забрал управление у агента");
            self.core.agent_cooldown = Some(Instant::now() + COOLDOWN);
        } else {
            tracing::info!("агент больше не управляет");
        }
        self.core.agent_keys.clear();
        self.agent_broadcast();
    }

    fn agent_broadcast(&mut self) {
        let agent = self.agent_info();
        self.core.ipc.broadcast(&Event::Agent { agent });
    }

    /// Физический ввод пользователя: запомнить время; при полном управлении
    /// агента — не пускать к окнам (кроме Ctrl+Alt+Esc). `true` — событие съедено.
    pub fn agent_filter<B: InputBackend>(&mut self, event: &InputEvent<B>) -> bool {
        if self.core.remote_input_active {
            return false;
        }
        let user = matches!(
            event,
            InputEvent::Keyboard { .. }
                | InputEvent::PointerMotion { .. }
                | InputEvent::PointerMotionAbsolute { .. }
                | InputEvent::PointerButton { .. }
                | InputEvent::PointerAxis { .. }
                | InputEvent::TouchDown { .. }
                | InputEvent::TouchMotion { .. }
                | InputEvent::TouchUp { .. }
                | InputEvent::TouchFrame { .. }
                | InputEvent::TouchCancel { .. }
                | InputEvent::TabletToolAxis { .. }
                | InputEvent::TabletToolTip { .. }
                | InputEvent::TabletToolButton { .. }
                | InputEvent::TabletToolProximity { .. }
                | InputEvent::GestureSwipeBegin { .. }
                | InputEvent::GestureSwipeUpdate { .. }
                | InputEvent::GestureSwipeEnd { .. }
                | InputEvent::GesturePinchBegin { .. }
                | InputEvent::GesturePinchUpdate { .. }
                | InputEvent::GesturePinchEnd { .. }
                | InputEvent::GestureHoldBegin { .. }
                | InputEvent::GestureHoldEnd { .. }
        );
        if !user {
            return false;
        }
        self.core.last_physical = Instant::now();
        let blocking = self.core.agent.as_ref().is_some_and(|a| a.full && a.until > Instant::now());
        if !blocking {
            return false;
        }
        if let InputEvent::Keyboard { event } = event {
            let code = event.key_code().raw();
            if event.state() == KeyState::Pressed {
                self.core.agent_keys.insert(code);
                let held = |c: [u32; 2]| c.iter().any(|k| self.core.agent_keys.contains(k));
                if code == XKB_ESC && held(XKB_CTRL) && held(XKB_ALT) {
                    self.agent_end(true);
                }
            } else {
                self.core.agent_keys.remove(&code);
            }
        }
        true
    }
}
