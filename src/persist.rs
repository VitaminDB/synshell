//! Сохранение рекорда и статистики в простой текстовый файл.
//!
//! Путь: `$XDG_DATA_HOME/elle_block/stats.txt` или `~/.local/share/...`.
//! Без каталога данных (например, на Android без HOME) — работает в памяти.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub best: i32,
    pub games: u32,
    pub best_combo: u32,
    pub total_lines: u32,
    pub total_score: i64,
    pub perfects: u32,
    pub sound: bool,
    pub music: bool,
    /// Выбранная сложность (индекс `Difficulty`).
    pub difficulty: u8,
    /// Рекорды по сложностям: легко / норма / сложно.
    pub best_by: [i32; 3],
    /// Выбранный трек.
    pub track: u8,
    /// Треки по кругу (после конца трека — следующий).
    pub loop_all: bool,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            best: 0,
            games: 0,
            best_combo: 0,
            total_lines: 0,
            total_score: 0,
            perfects: 0,
            sound: true,
            music: true,
            difficulty: 1,
            best_by: [0; 3],
            track: 0,
            loop_all: false,
        }
    }
}

impl Stats {
    pub fn to_text(&self) -> String {
        format!(
            "best={}\ngames={}\nbest_combo={}\ntotal_lines={}\ntotal_score={}\nperfects={}\nsound={}\nmusic={}\ndifficulty={}\nbest_easy={}\nbest_normal={}\nbest_hard={}\ntrack={}\nloop_all={}\n",
            self.best, self.games, self.best_combo, self.total_lines, self.total_score, self.perfects,
            self.sound as u8, self.music as u8, self.difficulty, self.best_by[0], self.best_by[1], self.best_by[2], self.track,
            self.loop_all as u8
        )
    }

    pub fn from_text(text: &str) -> Self {
        let mut s = Stats::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "best" => s.best = v.parse().unwrap_or(0),
                "games" => s.games = v.parse().unwrap_or(0),
                "best_combo" => s.best_combo = v.parse().unwrap_or(0),
                "total_lines" => s.total_lines = v.parse().unwrap_or(0),
                "total_score" => s.total_score = v.parse().unwrap_or(0),
                "perfects" => s.perfects = v.parse().unwrap_or(0),
                "sound" => s.sound = v != "0",
                "music" => s.music = v != "0",
                "difficulty" => s.difficulty = v.parse::<u8>().unwrap_or(1).min(2),
                "best_easy" => s.best_by[0] = v.parse().unwrap_or(0),
                "best_normal" => s.best_by[1] = v.parse().unwrap_or(0),
                "best_hard" => s.best_by[2] = v.parse().unwrap_or(0),
                "track" => s.track = v.parse().unwrap_or(0),
                "loop_all" => s.loop_all = v != "0",
                _ => {}
            }
        }
        s
    }
}

pub fn path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ELLE_BLOCK_DATA") {
        return Some(PathBuf::from(p).join("stats.txt"));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("elle_block").join("stats.txt"))
}

pub fn load() -> Stats {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| Stats::from_text(&t))
        .unwrap_or_default()
}

pub fn save(stats: &Stats) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, stats.to_text());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Stats { best: 1234, games: 7, best_combo: 5, total_lines: 88, total_score: 9999, perfects: 2, sound: false, music: true, difficulty: 2, best_by: [10, 1234, 77], track: 3, loop_all: true };
        assert_eq!(Stats::from_text(&s.to_text()), s);
        assert_eq!(Stats::from_text("garbage\nbest=x\n"), Stats::default());
    }
}
