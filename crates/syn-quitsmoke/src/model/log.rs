//! Журнал выкуренных сигарет (и срывов).

use serde::{Deserialize, Serialize};

/// Одна запись журнала.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Момент (epoch ms).
    pub at: u64,
    /// Была ли сигарета «в графике» (для тейпера: после `next_allowed`).
    /// Для режима «сразу» любая запись — срыв (`allowed = false`).
    pub allowed: bool,
    /// Опциональная заметка.
    pub note: Option<String>,
}

/// Журнал.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CigaretteLog {
    pub entries: Vec<LogEntry>,
}

impl CigaretteLog {
    pub fn last(&self) -> Option<&LogEntry> {
        self.entries.last()
    }

    pub fn last_at(&self) -> Option<u64> {
        self.entries.last().map(|e| e.at)
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// Добавить запись.
    pub fn push(&mut self, at: u64, allowed: bool) {
        self.entries.push(LogEntry {
            at,
            allowed,
            note: None,
        });
    }

    /// Сколько записей в полуинтервале `[since, until)`.
    pub fn count_in(&self, since: u64, until: u64) -> usize {
        self.entries
            .iter()
            .filter(|e| e.at >= since && e.at < until)
            .count()
    }

    /// Сколько записей начиная с `since`.
    pub fn count_since(&self, since: u64) -> usize {
        self.entries.iter().filter(|e| e.at >= since).count()
    }

    /// Момент последнего срыва (`allowed == false`).
    pub fn last_slip_at(&self) -> Option<u64> {
        self.entries
            .iter()
            .rev()
            .find(|e| !e.allowed)
            .map(|e| e.at)
    }
}
