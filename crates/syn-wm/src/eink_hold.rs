//! E Ink: один кадр после поворота или смены масштаба вывода.
//!
//! Каждый кадр с изменениями — волна по панели, а почти полный кадр — полная GC-волна с морганием.
//! Сразу после смены transform клиенты ещё не перерисовались, и кадр показал бы старое содержимое
//! в новой ориентации (поверх цвета обоев), а следующий — уже их новые буферы: два моргания.
//! Поэтому вывод «держится»: кадры в DRM не уходят, пока все видимые поверхности на нём (слои
//! оболочки, окна, поверхность блокировки) не подтвердили последний отправленный им configure
//! коммитом (ack + commit) или не прошёл [`TIMEOUT`]. Потом — один кадр целиком.
//!
//! Пока вывод держится, клиентам идут frame callbacks (как в кадре без изменений): тулкиты,
//! которые рисуют только по frame callback, иначе не перерисовались бы.

use std::time::{Duration, Instant};

use smithay::{desktop::layer_map_for_output, output::Output};

use crate::state::Core;

/// Сколько ждать клиентов после поворота, прежде чем показать кадр как есть.
pub const TIMEOUT: Duration = Duration::from_millis(400);

/// Вывод держится до перерисовки клиентов.
#[derive(Debug, Clone)]
pub struct EinkHold {
    pub since: Instant,
    /// Что сменилось: «поворот», «масштаб», «режим».
    pub reason: &'static str,
    /// Сколько поверхностей ждали (наибольшее из замеров).
    pub waited: usize,
}

/// Решение для очередного кадра держащегося вывода.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Кадр не отдавать.
    Wait,
    /// Отпустить: кадр через `ms` мс, ждали `waited` поверхностей, не дождались `left`.
    Release { ms: u64, waited: usize, left: usize },
}

impl EinkHold {
    pub fn new(reason: &'static str, now: Instant) -> Self {
        Self { since: now, reason, waited: 0 }
    }

    /// `pending` — сколько видимых поверхностей ещё не закоммитили новый размер.
    pub fn step(&mut self, now: Instant, pending: usize) -> Step {
        self.waited = self.waited.max(pending);
        let elapsed = now.saturating_duration_since(self.since);
        if pending > 0 && elapsed < TIMEOUT {
            Step::Wait
        } else {
            Step::Release { ms: elapsed.as_millis() as u64, waited: self.waited, left: pending }
        }
    }
}

/// Поверхность ещё не закоммитила отправленный ей размер: `target` — последний отправленный
/// (или ещё не отправленный) размер configure, `current` — подтверждённый коммитом.
/// Без размера в configure (клиент выбирает сам) или ни разу не закоммиченная — не ждём.
pub fn unsettled<T: PartialEq>(target: Option<T>, current: Option<T>) -> bool {
    matches!((target, current), (Some(t), Some(c)) if t != c)
}

impl Core {
    /// Начать (или начать заново) держать вывод `output` — только на E Ink.
    pub fn start_eink_hold(&mut self, output: &Output, reason: &'static str) {
        if !self.eink {
            return;
        }
        let d = self.output_data.entry(output.clone()).or_default();
        let waited = d.eink_hold.as_ref().map(|h| h.waited).unwrap_or(0);
        let mut hold = EinkHold::new(reason, Instant::now());
        hold.waited = waited;
        d.eink_hold = Some(hold);
        tracing::debug!(output = output.name(), reason, "E Ink: кадры придержаны до перерисовки клиентов");
        // Таймаут: разбудить вывод, даже если никто так и не закоммитил.
        let o = output.clone();
        let _ = self.loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(TIMEOUT + Duration::from_millis(5)),
            move |_, _, state: &mut crate::state::State| {
                if state.core.output_data.get(&o).is_some_and(|d| d.eink_hold.is_some()) {
                    state.core.queue_redraw(&o);
                }
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        );
    }

    /// Сколько видимых на выводе поверхностей ещё не закоммитили последний configure с размером.
    pub fn eink_hold_pending(&self, output: &Output) -> usize {
        let geo = self.space.output_geometry(output).unwrap_or_default();
        if self.is_locked() {
            // На заблокированном экране видна только поверхность блокировки.
            return self
                .lock_surfaces
                .get(output)
                .filter(|ls| ls.alive())
                .map(|ls| {
                    let want = Some(smithay::utils::Size::<u32, smithay::utils::Logical>::from((geo.size.w as u32, geo.size.h as u32)));
                    unsettled(want, ls.current_state().size) as usize
                })
                .unwrap_or(0);
        }
        let mut n = 0;
        for w in self.space.elements() {
            if !self.space.element_geometry(w).is_some_and(|g| g.overlaps(geo)) {
                continue;
            }
            // X11-окна (Xwayland) configure по serial не подтверждают — их не ждём.
            let Some(t) = w.toplevel() else { continue };
            if unsettled(t.with_pending_state(|s| s.size), t.current_state().size) {
                n += 1;
            }
        }
        let map = layer_map_for_output(output);
        for l in map.layers() {
            let s = l.layer_surface();
            if unsettled(s.with_pending_state(|p| p.size), s.current_state().size) {
                n += 1;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsettled_sizes() {
        assert!(unsettled(Some((1860, 1240)), Some((1240, 1860))));
        assert!(!unsettled(Some((1860, 1240)), Some((1860, 1240))));
        // клиент выбирает размер сам / ещё не показан — не ждём
        assert!(!unsettled(None, Some((1240, 1860))));
        assert!(!unsettled(Some((1860, 1240)), None::<(i32, i32)>));
    }

    #[test]
    fn waits_for_clients_then_releases_once() {
        let t0 = Instant::now();
        let mut h = EinkHold::new("поворот", t0);
        // сразу после поворота: оболочка и окно ещё старого размера
        assert_eq!(h.step(t0, 2), Step::Wait);
        assert_eq!(h.step(t0 + Duration::from_millis(30), 1), Step::Wait);
        // все закоммитили — кадр, ждали двоих
        assert_eq!(h.step(t0 + Duration::from_millis(80), 0), Step::Release { ms: 80, waited: 2, left: 0 });
    }

    #[test]
    fn no_surfaces_releases_immediately() {
        let t0 = Instant::now();
        let mut h = EinkHold::new("поворот", t0);
        assert_eq!(h.step(t0, 0), Step::Release { ms: 0, waited: 0, left: 0 });
    }

    #[test]
    fn timeout_releases_with_stragglers() {
        let t0 = Instant::now();
        let mut h = EinkHold::new("поворот", t0);
        assert_eq!(h.step(t0, 3), Step::Wait);
        assert_eq!(h.step(t0 + TIMEOUT - Duration::from_millis(1), 1), Step::Wait);
        assert_eq!(h.step(t0 + TIMEOUT, 1), Step::Release { ms: TIMEOUT.as_millis() as u64, waited: 3, left: 1 });
    }
}
