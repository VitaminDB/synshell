//! Закрытие окна из оболочки (шторка, «Недавние», панель задач): просьба
//! закрыться, а если окно не ушло — показать его, повторное «закрыть» — убить.
//!
//! 1. Просьба закрыться (xdg_toplevel.close / WM_DELETE_WINDOW).
//! 2. Окно исчезло за [`SURFACE_AFTER`] — готово (в том числе «закрытие в трей»:
//!    окно ушло, процесс живёт — больше ничего не делаем).
//! 3. Окно осталось — развернуть и вывести вперёд с фокусом, что бы его ни держало:
//!    вопрос «Выйти?» часто нарисован внутри той же поверхности (syngui, Firefox,
//!    Electron), и свёрнутым его не видно. Новое окно-ребёнок (диалог) после
//!    просьбы — выводится сразу.
//! 4. Повторное «закрыть» в течение [`FORCE_WINDOW`] после первого — принудительно:
//!    SIGTERM процессу, через [`KILL_AFTER`] — SIGKILL, если он ещё жив.
//!
//! Здесь только учёт и сроки (без smithay) — их проверяют юнит-тесты; действия
//! выполняет `State` (`ops.rs`).

use std::time::{Duration, Instant};

use super::WindowId;

/// Сколько ждать, пока окно уйдёт само, прежде чем вывести его вперёд.
pub const SURFACE_AFTER: Duration = Duration::from_millis(700);
/// Сколько после первой просьбы повторное «закрыть» считается принудительным.
pub const FORCE_WINDOW: Duration = Duration::from_secs(30);
/// Повтор быстрее этого — дрогнувший палец или двойной щелчок, не просьба убить.
pub const DEBOUNCE: Duration = Duration::from_millis(400);
/// Сколько ждать после SIGTERM до SIGKILL.
pub const KILL_AFTER: Duration = Duration::from_millis(2500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Просьба отправлена, ждём [`SURFACE_AFTER`].
    Asked,
    /// Окно не ушло и выведено вперёд.
    Surfaced,
}

#[derive(Debug, Clone)]
struct Entry {
    id: WindowId,
    asked: Instant,
    stage: Stage,
}

/// Что делать по «закрыть» из оболочки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Попросить окно закрыться.
    Ask,
    /// Окну уже отправляли просьбу — завершить процесс.
    Terminate,
    /// Повтор сразу за первой просьбой ([`DEBOUNCE`]) — пропустить.
    Ignore,
}

/// Что наступило к моменту [`CloseWatch::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// Окно не ушло — развернуть и вывести вперёд.
    Surface(WindowId),
    /// Срок принудительного закрытия вышел — следующее «закрыть» снова просьба.
    Expired(WindowId),
    /// Процесс после SIGTERM ещё жив? — SIGKILL.
    Kill(i32),
}

#[derive(Debug, Default)]
pub struct CloseWatch {
    entries: Vec<Entry>,
    /// Процессы, которым отправлен SIGTERM: (pid, когда).
    terms: Vec<(i32, Instant)>,
}

impl CloseWatch {
    /// «Закрыть» из оболочки: первая просьба или повторная (принудительно).
    pub fn request(&mut self, id: WindowId, now: Instant) -> Decision {
        if let Some(e) = self.entries.iter().find(|e| e.id == id) {
            let age = now.duration_since(e.asked);
            if age < DEBOUNCE {
                return Decision::Ignore;
            }
            if age < FORCE_WINDOW {
                return Decision::Terminate;
            }
        }
        self.entries.retain(|e| e.id != id);
        self.entries.push(Entry { id, asked: now, stage: Stage::Asked });
        Decision::Ask
    }

    /// Процессу окна отправлен SIGTERM.
    pub fn terminated(&mut self, id: WindowId, pid: i32, now: Instant) {
        self.entries.retain(|e| e.id != id);
        self.terms.retain(|(p, _)| *p != pid);
        self.terms.push((pid, now));
    }

    /// Окно исчезло (уничтожено или убрало буфер) — следить не за чем.
    pub fn gone(&mut self, id: WindowId) {
        self.entries.retain(|e| e.id != id);
    }

    /// Следующее «закрыть» для окна будет принудительным.
    pub fn is_pending(&self, id: WindowId, now: Instant) -> bool {
        self.entries.iter().any(|e| e.id == id && now.duration_since(e.asked) < FORCE_WINDOW)
    }

    pub fn stage(&self, id: WindowId) -> Option<Stage> {
        self.entries.iter().find(|e| e.id == id).map(|e| e.stage)
    }

    /// У окна `parent` появилось окно-ребёнок (диалог). Если `parent` ждёт
    /// закрытия и ещё не выведен — вывести сейчас (true), не дожидаясь срока.
    pub fn child_appeared(&mut self, parent: WindowId) -> bool {
        match self.entries.iter_mut().find(|e| e.id == parent) {
            Some(e) if e.stage == Stage::Asked => {
                e.stage = Stage::Surfaced;
                true
            }
            _ => false,
        }
    }

    /// Сроки, наступившие к `now`.
    pub fn tick(&mut self, now: Instant) -> Vec<Due> {
        let mut due = Vec::new();
        self.entries.retain_mut(|e| {
            let age = now.duration_since(e.asked);
            if age >= FORCE_WINDOW {
                due.push(Due::Expired(e.id));
                return false;
            }
            if e.stage == Stage::Asked && age >= SURFACE_AFTER {
                e.stage = Stage::Surfaced;
                due.push(Due::Surface(e.id));
            }
            true
        });
        self.terms.retain(|(pid, t)| {
            if now.duration_since(*t) >= KILL_AFTER {
                due.push(Due::Kill(*pid));
                false
            } else {
                true
            }
        });
        due
    }

    /// Ближайший срок (для таймера цикла событий).
    pub fn next_deadline(&self) -> Option<Instant> {
        let e = self.entries.iter().map(|e| match e.stage {
            Stage::Asked => e.asked + SURFACE_AFTER,
            Stage::Surfaced => e.asked + FORCE_WINDOW,
        });
        let t = self.terms.iter().map(|(_, t)| *t + KILL_AFTER);
        e.chain(t).min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn closed_in_time_nothing_happens() {
        let mut w = CloseWatch::default();
        let t0 = Instant::now();
        assert_eq!(w.request(1, t0), Decision::Ask);
        assert!(w.tick(t0 + ms(300)).is_empty());
        w.gone(1);
        assert!(w.tick(t0 + ms(800)).is_empty());
        assert!(!w.is_pending(1, t0 + ms(800)));
        assert_eq!(w.next_deadline(), None);
        // снова «закрыть» (окно с тем же id не вернётся, но учёт чистый) — просьба
        assert_eq!(w.request(1, t0 + ms(900)), Decision::Ask);
    }

    #[test]
    fn stays_open_is_surfaced_once() {
        let mut w = CloseWatch::default();
        let t0 = Instant::now();
        w.request(7, t0);
        assert_eq!(w.next_deadline(), Some(t0 + SURFACE_AFTER));
        assert!(w.tick(t0 + ms(699)).is_empty());
        assert_eq!(w.tick(t0 + ms(700)), vec![Due::Surface(7)]);
        assert_eq!(w.stage(7), Some(Stage::Surfaced));
        // второй раз не выводим
        assert!(w.tick(t0 + ms(1500)).is_empty());
        assert!(w.is_pending(7, t0 + ms(1500)));
        assert_eq!(w.next_deadline(), Some(t0 + FORCE_WINDOW));
    }

    #[test]
    fn second_close_terminates_then_kills() {
        let mut w = CloseWatch::default();
        let t0 = Instant::now();
        w.request(3, t0);
        w.tick(t0 + ms(800));
        let t1 = t0 + ms(2000);
        assert_eq!(w.request(3, t1), Decision::Terminate);
        w.terminated(3, 4242, t1);
        assert!(!w.is_pending(3, t1));
        assert_eq!(w.next_deadline(), Some(t1 + KILL_AFTER));
        assert!(w.tick(t1 + ms(2400)).is_empty());
        assert_eq!(w.tick(t1 + KILL_AFTER), vec![Due::Kill(4242)]);
        assert!(w.tick(t1 + ms(5000)).is_empty());
    }

    #[test]
    fn second_close_before_surfacing_is_forced_too() {
        let mut w = CloseWatch::default();
        let t0 = Instant::now();
        w.request(3, t0);
        assert_eq!(w.request(3, t0 + ms(200)), Decision::Ignore);
        assert_eq!(w.request(3, t0 + ms(500)), Decision::Terminate);
    }

    #[test]
    fn force_window_expires() {
        let mut w = CloseWatch::default();
        let t0 = Instant::now();
        w.request(5, t0);
        w.tick(t0 + ms(800));
        assert_eq!(w.tick(t0 + FORCE_WINDOW), vec![Due::Expired(5)]);
        assert!(!w.is_pending(5, t0 + FORCE_WINDOW));
        assert_eq!(w.request(5, t0 + FORCE_WINDOW + ms(1)), Decision::Ask);
    }

    #[test]
    fn child_dialog_surfaces_immediately() {
        let mut w = CloseWatch::default();
        let t0 = Instant::now();
        w.request(9, t0);
        assert!(w.child_appeared(9));
        // уже выведено — по сроку не повторяем
        assert!(!w.child_appeared(9));
        assert!(w.tick(t0 + ms(800)).is_empty());
        // ребёнок окна, которое не закрывали, — не наше дело
        assert!(!w.child_appeared(10));
    }

    #[test]
    fn several_windows_independent() {
        let mut w = CloseWatch::default();
        let t0 = Instant::now();
        w.request(1, t0);
        w.request(2, t0 + ms(300));
        assert_eq!(w.tick(t0 + ms(700)), vec![Due::Surface(1)]);
        w.gone(2);
        assert!(w.tick(t0 + ms(1100)).is_empty());
        assert!(w.is_pending(1, t0 + ms(1100)));
    }
}
